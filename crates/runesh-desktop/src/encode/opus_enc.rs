//! Opus audio encoder.
//!
//! Wraps the `opus` crate to encode 20 ms PCM frames (Opus's default packet
//! length) into RTP-ready payloads. The WebRTC Opus payloader in `webrtc-rs`
//! expects one Opus packet per RTP packet, so we hand it the output bytes as-is.

use std::time::Duration;

use opus::{Application, Channels, Encoder as OpusEncoder};

use crate::error::DesktopError;

/// 48 kHz is the canonical Opus sample rate used by WebRTC.
pub const OPUS_SAMPLE_RATE_HZ: u32 = 48_000;

/// Default Opus frame length in milliseconds. WebRTC interoperates cleanly at 20 ms.
pub const OPUS_FRAME_MS: u32 = 20;

/// Number of PCM samples in one Opus frame, per channel.
pub const OPUS_FRAME_SAMPLES: usize =
    (OPUS_SAMPLE_RATE_HZ as usize * OPUS_FRAME_MS as usize) / 1000;

/// Encoded Opus packet ready for a `TrackLocalStaticSample`.
pub struct AudioSample {
    /// Opus packet payload (without RTP header).
    pub data: Vec<u8>,
    /// Sample duration — 20 ms when using [`OPUS_FRAME_MS`].
    pub duration: Duration,
}

pub struct OpusSampleEncoder {
    enc: OpusEncoder,
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
        let mut enc = OpusEncoder::new(OPUS_SAMPLE_RATE_HZ as i32, ch, Application::Audio)
            .map_err(|e| DesktopError::Encoding(format!("Opus init: {e}")))?;
        enc.set_bitrate(opus::Bitrate::Bits((bitrate_kbps * 1000) as i32))
            .map_err(|e| DesktopError::Encoding(format!("Opus bitrate: {e}")))?;
        enc.set_inband_fec(true).ok();
        // Max permissible packet size per libopus guidance (~4000 bytes).
        Ok(Self {
            enc,
            channels,
            buf: vec![0u8; 4000],
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
        let written = self
            .enc
            .encode(pcm, &mut self.buf)
            .map_err(|e| DesktopError::Encoding(format!("Opus encode: {e}")))?;
        Ok(AudioSample {
            data: self.buf[..written].to_vec(),
            duration: Duration::from_millis(OPUS_FRAME_MS as u64),
        })
    }
}
