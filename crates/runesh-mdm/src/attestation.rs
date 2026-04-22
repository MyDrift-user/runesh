//! Android Key Attestation parser.
//!
//! Android devices can include a `KeyDescription` ASN.1 extension on the
//! certificate issued by Android Keystore. The extension OID is
//! `1.3.6.1.4.1.11129.2.1.17`. Its value is a DER-encoded `OCTET STRING`
//! that wraps a `SEQUENCE` with the following shape (abridged; see
//! https://source.android.com/docs/security/features/keystore/attestation):
//!
//! ```asn1
//! KeyDescription ::= SEQUENCE {
//!     attestationVersion         INTEGER,
//!     attestationSecurityLevel   SecurityLevel,        -- ENUMERATED
//!     keyMintVersion             INTEGER,
//!     keyMintSecurityLevel       SecurityLevel,        -- ENUMERATED
//!     attestationChallenge       OCTET STRING,
//!     uniqueId                   OCTET STRING,
//!     softwareEnforced           AuthorizationList,    -- opaque SEQUENCE
//!     hardwareEnforced           AuthorizationList,    -- opaque SEQUENCE
//! }
//!
//! SecurityLevel ::= ENUMERATED {
//!     Software            (0),
//!     TrustedEnvironment  (1),
//!     StrongBox           (2),
//! }
//! ```
//!
//! This parser reads the top-level fields and leaves `softwareEnforced`
//! and `hardwareEnforced` as raw DER bytes. The caller that needs to
//! introspect specific authorization tags (e.g., `rootOfTrust`,
//! `attestationIdBrand`) can parse those blobs themselves with the same
//! `der` crate; wiring up every tag mapping doubles the crate surface
//! without adding library value, since Android adds new tags every year.
//!
//! What this crate does NOT do:
//!
//! - X.509 chain extraction and validation. The consumer must fetch the
//!   attestation certificate chain from the device, verify it up to
//!   Google's hardware attestation root, and hand the extracted
//!   extension bytes to [`parse_key_description`]. A reasonable chain
//!   validator is `x509-verify` or the `webpki` crate.
//! - Revocation checks against Google's key attestation status list.
//! - Authorization-tag interpretation (which OID maps to root-of-trust
//!   verified boot state, etc.).

use der::asn1::{ObjectIdentifier, OctetString};
use der::{Decode, Reader, SliceReader, Tag};

/// OID of the Android Key Attestation extension as it appears on the
/// certificate the device produces.
pub const ANDROID_KEY_ATTESTATION_OID: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.4.1.11129.2.1.17");

/// Security level as claimed by the attestation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityLevel {
    /// Software-only Keystore. No hardware guarantees.
    Software,
    /// Trusted Execution Environment (TEE). Keys live outside the
    /// application processor.
    TrustedEnvironment,
    /// Dedicated hardware (Titan M, iSE). Strongest guarantee.
    StrongBox,
    /// Any other enumerated value that might appear in a future spec.
    Unknown(i64),
}

impl SecurityLevel {
    fn from_enum(v: i64) -> Self {
        match v {
            0 => Self::Software,
            1 => Self::TrustedEnvironment,
            2 => Self::StrongBox,
            other => Self::Unknown(other),
        }
    }

    /// True for `TrustedEnvironment` or `StrongBox`. Useful for a quick
    /// "is this attestation hardware-backed" gate.
    pub fn is_hardware_backed(self) -> bool {
        matches!(self, Self::TrustedEnvironment | Self::StrongBox)
    }
}

/// Parsed KeyDescription. `software_enforced` and `hardware_enforced`
/// are left as raw DER bytes that the caller can re-parse if they need
/// specific authorization tags.
#[derive(Debug, Clone)]
pub struct KeyDescription {
    pub attestation_version: i64,
    pub attestation_security_level: SecurityLevel,
    pub keymint_version: i64,
    pub keymint_security_level: SecurityLevel,
    /// The nonce the relying party supplied at key generation time. A
    /// verifier MUST compare this byte-for-byte against its own
    /// expected challenge before trusting the attestation.
    pub attestation_challenge: Vec<u8>,
    /// Opaque uniqueId, empty on most devices.
    pub unique_id: Vec<u8>,
    pub software_enforced: Vec<u8>,
    pub hardware_enforced: Vec<u8>,
}

/// Errors produced by [`parse_key_description`].
#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("DER parse error: {0}")]
    Der(String),
    #[error("unexpected tag: expected {expected}, got {got}")]
    UnexpectedTag { expected: &'static str, got: String },
    #[error("integer does not fit in i64")]
    IntegerOverflow,
    #[error("trailing bytes after KeyDescription")]
    TrailingBytes,
}

impl From<der::Error> for AttestationError {
    fn from(e: der::Error) -> Self {
        Self::Der(e.to_string())
    }
}

/// Parse a KeyDescription from the DER bytes that live inside the
/// attestation extension's OCTET STRING. The caller is responsible for
/// extracting those bytes from the X.509 certificate.
pub fn parse_key_description(der_bytes: &[u8]) -> Result<KeyDescription, AttestationError> {
    let mut reader = SliceReader::new(der_bytes)?;
    let result = reader.sequence(|inner| {
        let attestation_version = read_i64(inner)?;
        let attestation_security_level = SecurityLevel::from_enum(read_enum(inner)?);
        let keymint_version = read_i64(inner)?;
        let keymint_security_level = SecurityLevel::from_enum(read_enum(inner)?);
        let attestation_challenge = OctetString::decode(inner)?.into_bytes();
        let unique_id = OctetString::decode(inner)?.into_bytes();
        let software_enforced = read_raw_sequence(inner)?;
        let hardware_enforced = read_raw_sequence(inner)?;
        Ok::<_, der::Error>(KeyDescription {
            attestation_version,
            attestation_security_level,
            keymint_version,
            keymint_security_level,
            attestation_challenge,
            unique_id,
            software_enforced,
            hardware_enforced,
        })
    })?;
    if !reader.is_finished() {
        return Err(AttestationError::TrailingBytes);
    }
    Ok(result)
}

fn read_i64<'a, R: Reader<'a>>(reader: &mut R) -> Result<i64, der::Error> {
    reader.decode::<i64>()
}

fn read_enum<'a, R: Reader<'a>>(reader: &mut R) -> Result<i64, der::Error> {
    // der 0.7 represents ENUMERATED as tag(10). Decode the header then
    // the value as big-endian signed integer, same shape as INTEGER.
    let header = der::Header::decode(reader)?;
    if header.tag != Tag::Enumerated {
        return Err(reader.error(header.tag.non_canonical_error().kind()));
    }
    let bytes = reader.read_slice(header.length)?;
    if bytes.is_empty() {
        return Ok(0);
    }
    let mut value: i64 = 0;
    // Sign-extend.
    if bytes[0] & 0x80 != 0 {
        value = -1;
    }
    for b in bytes {
        value = (value << 8) | (*b as i64);
    }
    Ok(value)
}

fn read_raw_sequence<'a, R: Reader<'a>>(reader: &mut R) -> Result<Vec<u8>, der::Error> {
    let header = der::Header::decode(reader)?;
    if header.tag != Tag::Sequence {
        return Err(reader
            .error(
                der::ErrorKind::TagUnexpected {
                    expected: Some(Tag::Sequence),
                    actual: header.tag,
                }
                .into(),
            )
            .kind()
            .into());
    }
    let body = reader.read_slice(header.length)?;
    // Re-emit the header so the returned bytes are a self-contained DER
    // SEQUENCE that the caller can re-parse.
    let mut out = Vec::with_capacity(body.len() + 4);
    header.encode_to_vec(&mut out)?;
    out.extend_from_slice(body);
    Ok(out)
}

/// Small helper: encode a [`der::Header`] to a Vec without touching the
/// `der::Writer` machinery. We need this because `header.encode_to_vec`
/// is not public in der 0.7.
trait HeaderEncode {
    fn encode_to_vec(&self, out: &mut Vec<u8>) -> Result<(), der::Error>;
}

impl HeaderEncode for der::Header {
    fn encode_to_vec(&self, out: &mut Vec<u8>) -> Result<(), der::Error> {
        // Tag byte: identifier octet.
        let tag_byte: u8 = self.tag.into();
        out.push(tag_byte);
        let len: u32 = self.length.into();
        if len < 128 {
            out.push(len as u8);
        } else if len < 256 {
            out.push(0x81);
            out.push(len as u8);
        } else if len < 65536 {
            out.push(0x82);
            out.push((len >> 8) as u8);
            out.push(len as u8);
        } else {
            out.push(0x83);
            out.push((len >> 16) as u8);
            out.push((len >> 8) as u8);
            out.push(len as u8);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-crafted minimal KeyDescription. All numeric fields are
    /// single-byte where possible so the offsets stay easy to audit.
    fn sample_key_description() -> Vec<u8> {
        // SEQUENCE body:
        //   INTEGER 300 (0x01 0x2C)
        //   ENUMERATED 1 (TrustedEnvironment)
        //   INTEGER 300
        //   ENUMERATED 1
        //   OCTET STRING [DE AD BE EF]
        //   OCTET STRING [00]
        //   SEQUENCE (empty)
        //   SEQUENCE (empty)
        let body: &[u8] = &[
            0x02, 0x02, 0x01, 0x2C, // attestationVersion = 300
            0x0A, 0x01, 0x01, // attestationSecurityLevel = TrustedEnvironment
            0x02, 0x02, 0x01, 0x2C, // keyMintVersion = 300
            0x0A, 0x01, 0x01, // keyMintSecurityLevel = TrustedEnvironment
            0x04, 0x04, 0xDE, 0xAD, 0xBE, 0xEF, // attestationChallenge
            0x04, 0x01, 0x00, // uniqueId = 0x00
            0x30, 0x00, // softwareEnforced = empty SEQUENCE
            0x30, 0x00, // hardwareEnforced = empty SEQUENCE
        ];
        let mut seq = Vec::with_capacity(body.len() + 2);
        seq.push(0x30);
        seq.push(body.len() as u8);
        seq.extend_from_slice(body);
        seq
    }

    #[test]
    fn oid_matches_android_spec() {
        assert_eq!(
            ANDROID_KEY_ATTESTATION_OID.to_string(),
            "1.3.6.1.4.1.11129.2.1.17"
        );
    }

    #[test]
    fn parse_minimal_key_description() {
        let bytes = sample_key_description();
        let kd = parse_key_description(&bytes).expect("parse");
        assert_eq!(kd.attestation_version, 300);
        assert_eq!(
            kd.attestation_security_level,
            SecurityLevel::TrustedEnvironment
        );
        assert_eq!(kd.keymint_version, 300);
        assert_eq!(kd.keymint_security_level, SecurityLevel::TrustedEnvironment);
        assert_eq!(kd.attestation_challenge, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(kd.unique_id, vec![0x00]);
        // Raw SEQUENCEs are the two-byte `30 00`.
        assert_eq!(kd.software_enforced, vec![0x30, 0x00]);
        assert_eq!(kd.hardware_enforced, vec![0x30, 0x00]);
    }

    #[test]
    fn security_level_hardware_gate() {
        assert!(SecurityLevel::TrustedEnvironment.is_hardware_backed());
        assert!(SecurityLevel::StrongBox.is_hardware_backed());
        assert!(!SecurityLevel::Software.is_hardware_backed());
        assert!(!SecurityLevel::Unknown(99).is_hardware_backed());
    }

    #[test]
    fn trailing_bytes_rejected() {
        let mut bytes = sample_key_description();
        bytes.push(0xFF); // extra byte after the outer SEQUENCE
        let err = parse_key_description(&bytes).unwrap_err();
        match err {
            AttestationError::TrailingBytes => {}
            other => panic!("expected TrailingBytes, got {other:?}"),
        }
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(parse_key_description(&[]).is_err());
        assert!(parse_key_description(&[0x00]).is_err());
        assert!(parse_key_description(&[0x30, 0x03, 0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn unknown_security_level_is_preserved() {
        // Same shape as the sample, but the first ENUMERATED is 7
        // (not a defined value).
        let body: &[u8] = &[
            0x02, 0x01, 0x01, // attestationVersion = 1
            0x0A, 0x01, 0x07, // attestationSecurityLevel = 7 (unknown)
            0x02, 0x01, 0x01, // keyMintVersion = 1
            0x0A, 0x01, 0x01, // keyMintSecurityLevel = 1
            0x04, 0x00, // attestationChallenge = empty
            0x04, 0x00, // uniqueId = empty
            0x30, 0x00, 0x30, 0x00,
        ];
        let mut seq = Vec::with_capacity(body.len() + 2);
        seq.push(0x30);
        seq.push(body.len() as u8);
        seq.extend_from_slice(body);
        let kd = parse_key_description(&seq).unwrap();
        assert_eq!(kd.attestation_security_level, SecurityLevel::Unknown(7));
    }
}
