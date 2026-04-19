//! DERP frame protocol.
//!
//! The DERP protocol is a simple framing format over TCP for relaying
//! encrypted WireGuard packets between peers. The relay never decrypts
//! the payload; it simply routes by WireGuard public key.
//!
//! Frame format:
//!   [type: 1 byte] [length: 4 bytes big-endian] [payload: length bytes]
//!
//! Reference: github.com/tailscale/tailscale/blob/main/derp/derp.go

use bytes::{Buf, BufMut, BytesMut};

use crate::RelayError;

/// Maximum frame payload size (64 KiB).
pub const MAX_FRAME_SIZE: usize = 64 * 1024;

/// WireGuard public key length.
pub const KEY_LEN: usize = 32;

/// DERP frame types (from Tailscale source).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Server sends its public key to the client.
    ServerKey = 0x01,
    /// Client sends its info (JSON) after authentication.
    ClientInfo = 0x02,
    /// Client sends a packet to be forwarded to a peer.
    SendPacket = 0x04,
    /// Server delivers a packet from another peer.
    RecvPacket = 0x05,
    /// Keepalive (no payload).
    KeepAlive = 0x06,
    /// Client declares this relay as its preferred/home relay.
    NotePreferred = 0x07,
    /// Server notifies that a peer has disconnected.
    PeerGone = 0x08,
    /// Server notifies that a peer is connected to this relay.
    PeerPresent = 0x09,
    /// Client subscribes to peer connection events.
    WatchConns = 0x0a,
    /// Server tells client to close connection to a specific peer.
    ClosePeer = 0x0b,
    /// Ping (payload: 8 bytes opaque data).
    Ping = 0x0c,
    /// Pong (payload: echoed 8 bytes from Ping).
    Pong = 0x0d,
    /// Server sends health status string.
    Health = 0x0e,
    /// Server is restarting, clients should reconnect.
    Restarting = 0x0f,
    /// Server-to-server forwarding (multi-relay mesh).
    ForwardPacket = 0x10,
}

impl FrameType {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::ServerKey),
            0x02 => Some(Self::ClientInfo),
            0x04 => Some(Self::SendPacket),
            0x05 => Some(Self::RecvPacket),
            0x06 => Some(Self::KeepAlive),
            0x07 => Some(Self::NotePreferred),
            0x08 => Some(Self::PeerGone),
            0x09 => Some(Self::PeerPresent),
            0x0a => Some(Self::WatchConns),
            0x0b => Some(Self::ClosePeer),
            0x0c => Some(Self::Ping),
            0x0d => Some(Self::Pong),
            0x0e => Some(Self::Health),
            0x0f => Some(Self::Restarting),
            0x10 => Some(Self::ForwardPacket),
            _ => None,
        }
    }
}

/// A parsed DERP frame.
#[derive(Debug, Clone)]
pub enum Frame {
    /// Server key announcement.
    ServerKey { key: [u8; KEY_LEN] },

    /// Client info (JSON-encoded).
    ClientInfo { data: Vec<u8> },

    /// Send a packet to a peer (client to server).
    /// First 32 bytes of payload = destination public key, rest = packet data.
    SendPacket {
        dst_key: [u8; KEY_LEN],
        data: Vec<u8>,
    },

    /// Receive a packet from a peer (server to client).
    /// First 32 bytes of payload = source public key, rest = packet data.
    RecvPacket {
        src_key: [u8; KEY_LEN],
        data: Vec<u8>,
    },

    /// Keepalive.
    KeepAlive,

    /// Mark this relay as preferred.
    NotePreferred { preferred: bool },

    /// Peer gone notification.
    PeerGone { key: [u8; KEY_LEN] },

    /// Peer present notification.
    PeerPresent { key: [u8; KEY_LEN] },

    /// Subscribe to peer events.
    WatchConns,

    /// Close connection to peer.
    ClosePeer { key: [u8; KEY_LEN] },

    /// Ping with 8-byte data.
    Ping { data: [u8; 8] },

    /// Pong echoing 8-byte data.
    Pong { data: [u8; 8] },

    /// Health status.
    Health { message: String },

    /// Server restarting.
    Restarting {
        reconnect_in_ms: u32,
        try_for_ms: u32,
    },
}

/// Encode a frame into a byte buffer.
pub fn encode_frame(frame: &Frame, buf: &mut BytesMut) {
    match frame {
        Frame::ServerKey { key } => {
            write_header(buf, FrameType::ServerKey, KEY_LEN);
            buf.put_slice(key);
        }
        Frame::ClientInfo { data } => {
            write_header(buf, FrameType::ClientInfo, data.len());
            buf.put_slice(data);
        }
        Frame::SendPacket { dst_key, data } => {
            write_header(buf, FrameType::SendPacket, KEY_LEN + data.len());
            buf.put_slice(dst_key);
            buf.put_slice(data);
        }
        Frame::RecvPacket { src_key, data } => {
            write_header(buf, FrameType::RecvPacket, KEY_LEN + data.len());
            buf.put_slice(src_key);
            buf.put_slice(data);
        }
        Frame::KeepAlive => {
            write_header(buf, FrameType::KeepAlive, 0);
        }
        Frame::NotePreferred { preferred } => {
            write_header(buf, FrameType::NotePreferred, 1);
            buf.put_u8(if *preferred { 1 } else { 0 });
        }
        Frame::PeerGone { key } => {
            write_header(buf, FrameType::PeerGone, KEY_LEN);
            buf.put_slice(key);
        }
        Frame::PeerPresent { key } => {
            write_header(buf, FrameType::PeerPresent, KEY_LEN);
            buf.put_slice(key);
        }
        Frame::WatchConns => {
            write_header(buf, FrameType::WatchConns, 0);
        }
        Frame::ClosePeer { key } => {
            write_header(buf, FrameType::ClosePeer, KEY_LEN);
            buf.put_slice(key);
        }
        Frame::Ping { data } => {
            write_header(buf, FrameType::Ping, 8);
            buf.put_slice(data);
        }
        Frame::Pong { data } => {
            write_header(buf, FrameType::Pong, 8);
            buf.put_slice(data);
        }
        Frame::Health { message } => {
            let bytes = message.as_bytes();
            write_header(buf, FrameType::Health, bytes.len());
            buf.put_slice(bytes);
        }
        Frame::Restarting {
            reconnect_in_ms,
            try_for_ms,
        } => {
            write_header(buf, FrameType::Restarting, 8);
            buf.put_u32(*reconnect_in_ms);
            buf.put_u32(*try_for_ms);
        }
    }
}

/// Decode a frame from a byte buffer.
///
/// Returns `None` if the buffer doesn't contain a complete frame yet.
pub fn decode_frame(buf: &mut BytesMut) -> Result<Option<Frame>, RelayError> {
    if buf.len() < 5 {
        return Ok(None);
    }

    let frame_type = buf[0];
    let length = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;

    if length > MAX_FRAME_SIZE {
        return Err(RelayError::FrameTooLarge(length, MAX_FRAME_SIZE));
    }

    if buf.len() < 5 + length {
        return Ok(None); // Need more data
    }

    // Consume the header
    buf.advance(5);
    let payload = buf.split_to(length);

    let ft = FrameType::from_u8(frame_type).ok_or_else(|| {
        RelayError::InvalidFrame(format!("unknown frame type: 0x{frame_type:02x}"))
    })?;

    let frame = match ft {
        FrameType::ServerKey => {
            let key = read_key(&payload)?;
            Frame::ServerKey { key }
        }
        FrameType::ClientInfo => Frame::ClientInfo {
            data: payload.to_vec(),
        },
        FrameType::SendPacket => {
            if payload.len() < KEY_LEN {
                return Err(RelayError::InvalidFrame("SendPacket too short".into()));
            }
            let dst_key = read_key(&payload[..KEY_LEN])?;
            Frame::SendPacket {
                dst_key,
                data: payload[KEY_LEN..].to_vec(),
            }
        }
        FrameType::RecvPacket => {
            if payload.len() < KEY_LEN {
                return Err(RelayError::InvalidFrame("RecvPacket too short".into()));
            }
            let src_key = read_key(&payload[..KEY_LEN])?;
            Frame::RecvPacket {
                src_key,
                data: payload[KEY_LEN..].to_vec(),
            }
        }
        FrameType::KeepAlive => Frame::KeepAlive,
        FrameType::NotePreferred => Frame::NotePreferred {
            preferred: payload.first().copied().unwrap_or(0) != 0,
        },
        FrameType::PeerGone => {
            let key = read_key(&payload)?;
            Frame::PeerGone { key }
        }
        FrameType::PeerPresent => {
            let key = read_key(&payload)?;
            Frame::PeerPresent { key }
        }
        FrameType::WatchConns => Frame::WatchConns,
        FrameType::ClosePeer => {
            let key = read_key(&payload)?;
            Frame::ClosePeer { key }
        }
        FrameType::Ping => {
            if payload.len() < 8 {
                return Err(RelayError::InvalidFrame("Ping needs 8 bytes".into()));
            }
            let mut data = [0u8; 8];
            data.copy_from_slice(&payload[..8]);
            Frame::Ping { data }
        }
        FrameType::Pong => {
            if payload.len() < 8 {
                return Err(RelayError::InvalidFrame("Pong needs 8 bytes".into()));
            }
            let mut data = [0u8; 8];
            data.copy_from_slice(&payload[..8]);
            Frame::Pong { data }
        }
        FrameType::Health => Frame::Health {
            message: String::from_utf8_lossy(&payload).into_owned(),
        },
        FrameType::Restarting => {
            if payload.len() < 8 {
                return Err(RelayError::InvalidFrame("Restarting needs 8 bytes".into()));
            }
            Frame::Restarting {
                reconnect_in_ms: u32::from_be_bytes([
                    payload[0], payload[1], payload[2], payload[3],
                ]),
                try_for_ms: u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
            }
        }
        FrameType::ForwardPacket => {
            // Same as SendPacket but server-to-server
            if payload.len() < KEY_LEN {
                return Err(RelayError::InvalidFrame("ForwardPacket too short".into()));
            }
            let dst_key = read_key(&payload[..KEY_LEN])?;
            Frame::SendPacket {
                dst_key,
                data: payload[KEY_LEN..].to_vec(),
            }
        }
    };

    Ok(Some(frame))
}

fn write_header(buf: &mut BytesMut, ft: FrameType, length: usize) {
    buf.put_u8(ft as u8);
    buf.put_u32(length as u32);
}

fn read_key(data: &[u8]) -> Result<[u8; KEY_LEN], RelayError> {
    data.try_into()
        .map_err(|_| RelayError::InvalidFrame(format!("expected {KEY_LEN} bytes for key")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keepalive() {
        let mut buf = BytesMut::new();
        encode_frame(&Frame::KeepAlive, &mut buf);
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        assert!(matches!(frame, Frame::KeepAlive));
        assert!(buf.is_empty());
    }

    #[test]
    fn roundtrip_send_packet() {
        let dst_key = [42u8; KEY_LEN];
        let data = b"hello encrypted wireguard packet".to_vec();
        let mut buf = BytesMut::new();
        encode_frame(
            &Frame::SendPacket {
                dst_key,
                data: data.clone(),
            },
            &mut buf,
        );
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        match frame {
            Frame::SendPacket {
                dst_key: k,
                data: d,
            } => {
                assert_eq!(k, dst_key);
                assert_eq!(d, data);
            }
            _ => panic!("wrong frame type"),
        }
    }

    #[test]
    fn roundtrip_recv_packet() {
        let src_key = [99u8; KEY_LEN];
        let data = b"response data".to_vec();
        let mut buf = BytesMut::new();
        encode_frame(
            &Frame::RecvPacket {
                src_key,
                data: data.clone(),
            },
            &mut buf,
        );
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        match frame {
            Frame::RecvPacket {
                src_key: k,
                data: d,
            } => {
                assert_eq!(k, src_key);
                assert_eq!(d, data);
            }
            _ => panic!("wrong frame type"),
        }
    }

    #[test]
    fn roundtrip_ping_pong() {
        let ping_data = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut buf = BytesMut::new();
        encode_frame(&Frame::Ping { data: ping_data }, &mut buf);
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        match frame {
            Frame::Ping { data } => assert_eq!(data, ping_data),
            _ => panic!("wrong frame type"),
        }

        encode_frame(&Frame::Pong { data: ping_data }, &mut buf);
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        match frame {
            Frame::Pong { data } => assert_eq!(data, ping_data),
            _ => panic!("wrong frame type"),
        }
    }

    #[test]
    fn roundtrip_server_key() {
        let key = [7u8; KEY_LEN];
        let mut buf = BytesMut::new();
        encode_frame(&Frame::ServerKey { key }, &mut buf);
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        match frame {
            Frame::ServerKey { key: k } => assert_eq!(k, key),
            _ => panic!("wrong frame type"),
        }
    }

    #[test]
    fn roundtrip_health() {
        let mut buf = BytesMut::new();
        encode_frame(
            &Frame::Health {
                message: "ok".into(),
            },
            &mut buf,
        );
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        match frame {
            Frame::Health { message } => assert_eq!(message, "ok"),
            _ => panic!("wrong frame type"),
        }
    }

    #[test]
    fn roundtrip_restarting() {
        let mut buf = BytesMut::new();
        encode_frame(
            &Frame::Restarting {
                reconnect_in_ms: 1000,
                try_for_ms: 30000,
            },
            &mut buf,
        );
        let frame = decode_frame(&mut buf).unwrap().unwrap();
        match frame {
            Frame::Restarting {
                reconnect_in_ms,
                try_for_ms,
            } => {
                assert_eq!(reconnect_in_ms, 1000);
                assert_eq!(try_for_ms, 30000);
            }
            _ => panic!("wrong frame type"),
        }
    }

    #[test]
    fn partial_read_returns_none() {
        let mut buf = BytesMut::new();
        encode_frame(&Frame::KeepAlive, &mut buf);
        // Remove last byte to simulate partial read
        buf.truncate(buf.len() - 1);
        // Actually keepalive has 0 payload, so 5 bytes total. Let's test with just 3 bytes.
        let mut partial = BytesMut::from(&[0x06, 0x00][..]);
        assert!(decode_frame(&mut partial).unwrap().is_none());
    }

    #[test]
    fn frame_too_large() {
        let mut buf = BytesMut::new();
        buf.put_u8(0x02); // ClientInfo
        buf.put_u32(MAX_FRAME_SIZE as u32 + 1);
        assert!(decode_frame(&mut buf).is_err());
    }

    #[test]
    fn unknown_frame_type() {
        let mut buf = BytesMut::new();
        buf.put_u8(0xFF);
        buf.put_u32(0);
        assert!(decode_frame(&mut buf).is_err());
    }

    #[test]
    fn multiple_frames_in_buffer() {
        let mut buf = BytesMut::new();
        encode_frame(&Frame::KeepAlive, &mut buf);
        encode_frame(&Frame::Ping { data: [1; 8] }, &mut buf);

        let f1 = decode_frame(&mut buf).unwrap().unwrap();
        assert!(matches!(f1, Frame::KeepAlive));

        let f2 = decode_frame(&mut buf).unwrap().unwrap();
        assert!(matches!(f2, Frame::Ping { .. }));

        assert!(buf.is_empty());
    }
}
