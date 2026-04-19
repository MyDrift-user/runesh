//! Noise IK handshake for TS2021 protocol.
//!
//! Tailscale's control protocol uses Noise_IK_25519_ChaChaPoly_BLAKE2s.
//! In the IK pattern:
//! - The client knows the server's public key beforehand (from GET /key)
//! - The client sends its static key in the first message
//! - Two messages complete the handshake
//! - After handshake, both sides have a transport channel
//!
//! Reference: github.com/tailscale/tailscale/blob/main/control/controlhttp/noise.go

use base64::Engine;
use snow::{Builder, HandshakeState, Keypair, TransportState};

use crate::CoordError;

/// The Noise protocol pattern used by Tailscale.
pub const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

/// Maximum Noise message size.
const MAX_MSG_SIZE: usize = 65535;

/// Server-side Noise keypair.
pub struct NoiseKeypair {
    pub keypair: Keypair,
}

impl NoiseKeypair {
    /// Generate a new server keypair.
    pub fn generate() -> Result<Self, CoordError> {
        let keypair = Builder::new(
            NOISE_PATTERN
                .parse()
                .map_err(|e| CoordError::Handshake(format!("invalid noise pattern: {e}")))?,
        )
        .generate_keypair()
        .map_err(|e| CoordError::Handshake(format!("keygen failed: {e}")))?;
        Ok(Self { keypair })
    }

    /// Create from an existing private key (for persistence).
    pub fn from_private(private: &[u8]) -> Result<Self, CoordError> {
        if private.len() != 32 {
            return Err(CoordError::Handshake("private key must be 32 bytes".into()));
        }
        // Derive public key by building a dummy handshake
        let builder = Builder::new(
            NOISE_PATTERN
                .parse()
                .map_err(|e| CoordError::Handshake(format!("invalid noise pattern: {e}")))?,
        );
        let _kp = builder
            .generate_keypair()
            .map_err(|e| CoordError::Handshake(format!("keygen failed: {e}")))?;

        // Reconstruct with the provided private key
        let mut priv_bytes = [0u8; 32];
        priv_bytes.copy_from_slice(private);

        // Generate public from private using x25519
        let secret = x25519_dalek::StaticSecret::from(priv_bytes);
        let public = x25519_dalek::PublicKey::from(&secret);

        Ok(Self {
            keypair: Keypair {
                private: priv_bytes.to_vec(),
                public: public.as_bytes().to_vec(),
            },
        })
    }

    /// Get the public key bytes.
    pub fn public_key(&self) -> &[u8] {
        &self.keypair.public
    }
}

impl std::fmt::Debug for NoiseKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoiseKeypair")
            .field(
                "public",
                &base64::engine::general_purpose::STANDARD.encode(&self.keypair.public),
            )
            .field("private", &"[redacted]")
            .finish()
    }
}

/// Server-side Noise handshake handler.
///
/// Processes the IK handshake where the server is the responder.
pub struct NoiseResponder {
    state: HandshakeState,
}

impl NoiseResponder {
    /// Create a new responder with the server's keypair.
    pub fn new(server_keypair: &NoiseKeypair) -> Result<Self, CoordError> {
        let state = Builder::new(
            NOISE_PATTERN
                .parse()
                .map_err(|e| CoordError::Handshake(format!("invalid noise pattern: {e}")))?,
        )
        .local_private_key(&server_keypair.keypair.private)
        .build_responder()
        .map_err(|e| CoordError::Handshake(format!("build responder: {e}")))?;

        Ok(Self { state })
    }

    /// Process the client's first message and produce the server's response.
    ///
    /// Returns (response_bytes, client_public_key).
    pub fn respond(&mut self, client_msg: &[u8]) -> Result<(Vec<u8>, Vec<u8>), CoordError> {
        // Read client's first message (contains their static public key)
        let mut payload_buf = vec![0u8; MAX_MSG_SIZE];
        let _payload_len = self
            .state
            .read_message(client_msg, &mut payload_buf)
            .map_err(|e| CoordError::Handshake(format!("read client msg: {e}")))?;

        // Extract client's static public key from the handshake state
        let client_pubkey = self
            .state
            .get_remote_static()
            .ok_or_else(|| CoordError::Handshake("no remote static key after read".into()))?
            .to_vec();

        // Write server's response
        let mut response_buf = vec![0u8; MAX_MSG_SIZE];
        let response_len = self
            .state
            .write_message(&[], &mut response_buf)
            .map_err(|e| CoordError::Handshake(format!("write server response: {e}")))?;

        Ok((response_buf[..response_len].to_vec(), client_pubkey))
    }

    /// Transition to transport mode after handshake completion.
    pub fn into_transport(self) -> Result<NoiseTransport, CoordError> {
        let transport = self
            .state
            .into_transport_mode()
            .map_err(|e| CoordError::Handshake(format!("transport mode: {e}")))?;
        Ok(NoiseTransport { state: transport })
    }
}

/// Noise transport channel for encrypted communication after handshake.
pub struct NoiseTransport {
    state: TransportState,
}

impl NoiseTransport {
    /// Encrypt a message.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CoordError> {
        let mut buf = vec![0u8; plaintext.len() + 16]; // 16 bytes AEAD tag
        let len = self
            .state
            .write_message(plaintext, &mut buf)
            .map_err(|e| CoordError::Handshake(format!("encrypt: {e}")))?;
        buf.truncate(len);
        Ok(buf)
    }

    /// Decrypt a message.
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, CoordError> {
        let mut buf = vec![0u8; ciphertext.len()];
        let len = self
            .state
            .read_message(ciphertext, &mut buf)
            .map_err(|e| CoordError::Handshake(format!("decrypt: {e}")))?;
        buf.truncate(len);
        Ok(buf)
    }
}

/// Client-side Noise handshake initiator (for testing).
pub struct NoiseInitiator {
    state: HandshakeState,
}

impl NoiseInitiator {
    /// Create a new initiator with a client keypair and the server's public key.
    pub fn new(client_keypair: &Keypair, server_pubkey: &[u8]) -> Result<Self, CoordError> {
        let state = Builder::new(
            NOISE_PATTERN
                .parse()
                .map_err(|e| CoordError::Handshake(format!("invalid noise pattern: {e}")))?,
        )
        .local_private_key(&client_keypair.private)
        .remote_public_key(server_pubkey)
        .build_initiator()
        .map_err(|e| CoordError::Handshake(format!("build initiator: {e}")))?;

        Ok(Self { state })
    }

    /// Create the first handshake message.
    pub fn initiate(&mut self) -> Result<Vec<u8>, CoordError> {
        let mut buf = vec![0u8; MAX_MSG_SIZE];
        let len = self
            .state
            .write_message(&[], &mut buf)
            .map_err(|e| CoordError::Handshake(format!("write init msg: {e}")))?;
        Ok(buf[..len].to_vec())
    }

    /// Process the server's response.
    pub fn finish(&mut self, server_msg: &[u8]) -> Result<(), CoordError> {
        let mut buf = vec![0u8; MAX_MSG_SIZE];
        self.state
            .read_message(server_msg, &mut buf)
            .map_err(|e| CoordError::Handshake(format!("read server response: {e}")))?;
        Ok(())
    }

    /// Transition to transport mode.
    pub fn into_transport(self) -> Result<NoiseTransport, CoordError> {
        let transport = self
            .state
            .into_transport_mode()
            .map_err(|e| CoordError::Handshake(format!("transport mode: {e}")))?;
        Ok(NoiseTransport { state: transport })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_keypair() {
        let kp = NoiseKeypair::generate().unwrap();
        assert_eq!(kp.public_key().len(), 32);
    }

    #[test]
    fn full_handshake() {
        let server_kp = NoiseKeypair::generate().unwrap();
        let client_kp = Builder::new(NOISE_PATTERN.parse().unwrap())
            .generate_keypair()
            .unwrap();

        // Client initiates
        let mut initiator = NoiseInitiator::new(&client_kp, server_kp.public_key()).unwrap();
        let msg1 = initiator.initiate().unwrap();

        // Server responds
        let mut responder = NoiseResponder::new(&server_kp).unwrap();
        let (msg2, client_pubkey) = responder.respond(&msg1).unwrap();
        assert_eq!(client_pubkey, client_kp.public);

        // Client finishes
        initiator.finish(&msg2).unwrap();

        // Both transition to transport
        let mut client_transport = initiator.into_transport().unwrap();
        let mut server_transport = responder.into_transport().unwrap();

        // Client sends encrypted message
        let encrypted = client_transport.encrypt(b"hello server").unwrap();
        let decrypted = server_transport.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, b"hello server");

        // Server sends encrypted message
        let encrypted = server_transport.encrypt(b"hello client").unwrap();
        let decrypted = client_transport.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, b"hello client");
    }

    #[test]
    fn multiple_messages() {
        let server_kp = NoiseKeypair::generate().unwrap();
        let client_kp = Builder::new(NOISE_PATTERN.parse().unwrap())
            .generate_keypair()
            .unwrap();

        let mut initiator = NoiseInitiator::new(&client_kp, server_kp.public_key()).unwrap();
        let msg1 = initiator.initiate().unwrap();

        let mut responder = NoiseResponder::new(&server_kp).unwrap();
        let (msg2, _) = responder.respond(&msg1).unwrap();
        initiator.finish(&msg2).unwrap();

        let mut ct = initiator.into_transport().unwrap();
        let mut st = responder.into_transport().unwrap();

        // Send many messages both directions
        for i in 0..10 {
            let msg = format!("message {i}");
            let enc = ct.encrypt(msg.as_bytes()).unwrap();
            let dec = st.decrypt(&enc).unwrap();
            assert_eq!(dec, msg.as_bytes());

            let reply = format!("reply {i}");
            let enc = st.encrypt(reply.as_bytes()).unwrap();
            let dec = ct.decrypt(&enc).unwrap();
            assert_eq!(dec, reply.as_bytes());
        }
    }

    #[test]
    fn wrong_server_key_fails() {
        let server_kp = NoiseKeypair::generate().unwrap();
        let wrong_kp = NoiseKeypair::generate().unwrap();
        let client_kp = Builder::new(NOISE_PATTERN.parse().unwrap())
            .generate_keypair()
            .unwrap();

        // Client uses wrong server key
        let mut initiator = NoiseInitiator::new(&client_kp, wrong_kp.public_key()).unwrap();
        let msg1 = initiator.initiate().unwrap();

        // Server tries to respond -- should fail because client encrypted to wrong key
        let mut responder = NoiseResponder::new(&server_kp).unwrap();
        assert!(responder.respond(&msg1).is_err());
    }

    #[test]
    fn tampered_message_fails() {
        let server_kp = NoiseKeypair::generate().unwrap();
        let client_kp = Builder::new(NOISE_PATTERN.parse().unwrap())
            .generate_keypair()
            .unwrap();

        let mut initiator = NoiseInitiator::new(&client_kp, server_kp.public_key()).unwrap();
        let msg1 = initiator.initiate().unwrap();

        let mut responder = NoiseResponder::new(&server_kp).unwrap();
        let (msg2, _) = responder.respond(&msg1).unwrap();
        initiator.finish(&msg2).unwrap();

        let mut ct = initiator.into_transport().unwrap();
        let mut st = responder.into_transport().unwrap();

        let mut encrypted = ct.encrypt(b"secret").unwrap();
        // Tamper with ciphertext
        if let Some(byte) = encrypted.last_mut() {
            *byte ^= 0xFF;
        }
        assert!(st.decrypt(&encrypted).is_err());
    }
}
