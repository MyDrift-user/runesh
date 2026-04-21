//! Opus audio encoder, backed by the pure-Rust `mousiki` crate.
//!
//! Chosen over the various `libopus` binding crates because those all
//! transitively require system cmake + libclang + llvm-tools, which is
//! exactly the kind of build-environment coupling we want to avoid.
//! `mousiki` is a Rust port of the Xiph Opus C reference implementation
//! and only pulls in pure-Rust deps.
//!
//! 20 ms PCM frames at 48 kHz are the WebRTC interop baseline — each produced
//! `AudioSample` maps 1:1 to one RTP packet on the Opus track.

use std::time::Duration;

use mousiki::{Application, Bitrate, Channels, Encoder};

use crate::error::DesktopError;

/// 48 kHz is the canonical Opus sample rate used by WebRTC.
pub const OPUS_SAMPLE_RATE_HZ: u32 = 48_000;

/// Default Opus frame length in milliseconds. WebRTC interoperates cleanly at 20 ms.
pub const OPUS_FRAME_MS: u32 = 20;

/// Number of PCM samples in one Opus frame, per channel.
pub const OPUS_FRAME_SAMPLES: usize =
    (OPUS_SAMPLE_RATE_HZ as usize * OPUS_FRAME_MS as usize) / 1000;

/// Upper bound on a single Opus packet per the spec.
const MAX_OPUS_PACKET_BYTES: usize = 4000;

/// Encoded Opus packet ready for a `TrackLocalStaticSample`.
pub struct AudioSample {
    /// Opus packet payload (without RTP header).
    pub data: Vec<u8>,
    /// Sample duration — 20 ms when using [`OPUS_FRAME_MS`].
    pub duration: Duration,
}

pub struct OpusSampleEncoder {
    enc: Encoder,
    channels: u32,
    buf: Vec<u8>,
}

impl OpusSampleEncoder {
    pub fn new(channels: u32, bitrate_kbps: u32) -> Result<Self, DesktopError> {
        let ch = match channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            other => {
                return Err(DesktopError::Encoding(format!(
                    "Opus supports 1 or 2 channels, got {other}"
                )));
            }
        };
        let mut enc = Encoder::new(OPUS_SAMPLE_RATE_HZ, ch, Application::Audio)
            .map_err(|e| DesktopError::Encoding(format!("Opus init: {e:?}")))?;
        enc.set_bitrate(Bitrate::Bits((bitrate_kbps * 1000) as i32))
            .map_err(|e| DesktopError::Encoding(format!("Opus bitrate: {e:?}")))?;
        // Forward error correction pads low-rate packets with redundancy so the
        // viewer can recover from single-packet loss. Best-effort — ignored on
        // error since the encoder still works without it.
        let _ = enc.set_inband_fec(true);
        Ok(Self {
            enc,
            channels,
            buf: vec![0u8; MAX_OPUS_PACKET_BYTES],
        })
    }

    /// Encode exactly one 20 ms PCM frame (`OPUS_FRAME_SAMPLES * channels` i16 samples).
    pub fn encode_frame(&mut self, pcm: &[i16]) -> Result<AudioSample, DesktopError> {
        let expected = OPUS_FRAME_SAMPLES * self.channels as usize;
        if pcm.len() != expected {
            return Err(DesktopError::Encoding(format!(
                "Opus frame size mismatch: got {}, expected {}",
                pcm.len(),
                expected
            )));
        }
        let len = self
            .enc
            .encode(pcm, &mut self.buf)
            .map_err(|e| DesktopError::Encoding(format!("Opus encode: {e:?}")))?;
        Ok(AudioSample {
            data: self.buf[..len].to_vec(),
            duration: Duration::from_millis(OPUS_FRAME_MS as u64),
        })
    }
}
