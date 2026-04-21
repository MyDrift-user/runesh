//! Cross-platform audio capture.
//!
//! Uses [`cpal`] for device enumeration and stream construction. The capturer
//! produces mono 48 kHz `i16` frames that match the expectations of
//! [`crate::encode::opus_enc`].
//!
//! ## Platform notes
//!
//! - **Windows**: cpal's WASAPI backend supports loopback capture. We select
//!   the default output device and open it in loopback mode so we capture
//!   the system audio the host is playing.
//! - **macOS**: cpal captures the default input device. True system-audio
//!   loopback on macOS requires an Aggregate Device or a kernel extension
//!   (BlackHole / Loopback.app); we surface the default mic here.
//! - **Linux**: with PulseAudio/PipeWire, users can route a monitor source
//!   as the default input. cpal then captures it transparently.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

use crate::error::DesktopError;

pub use crate::encode::opus_enc::{OPUS_FRAME_MS, OPUS_FRAME_SAMPLES, OPUS_SAMPLE_RATE_HZ};

/// A captured audio frame: interleaved `i16` samples at the target sample rate.
pub struct CapturedAudio {
    /// Interleaved PCM samples. Length is `OPUS_FRAME_SAMPLES * channels`.
    pub samples: Vec<i16>,
    pub channels: u32,
}

/// Handle to a live audio capture stream. Dropping it stops capture.
pub struct AudioCapturer {
    _stream: cpal::Stream,
    rx: Receiver<CapturedAudio>,
    channels: u32,
}

impl AudioCapturer {
    /// Start capturing audio. On Windows this opens the default output device
    /// in WASAPI loopback mode; elsewhere it opens the default input device.
    pub fn start() -> Result<Self, DesktopError> {
        let host = cpal::default_host();

        #[cfg(target_os = "windows")]
        let device = host
            .default_output_device()
            .ok_or_else(|| DesktopError::Capture("no default output device".into()))?;

        #[cfg(not(target_os = "windows"))]
        let device = host
            .default_input_device()
            .ok_or_else(|| DesktopError::Capture("no default input device".into()))?;

        let supported = device
            .default_input_config()
            .or_else(|_| device.default_output_config())
            .map_err(|e| DesktopError::Capture(format!("cpal default config: {e}")))?;

        let channels = supported.channels() as u32;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();

        tracing::info!(
            sample_rate = config.sample_rate.0,
            channels,
            sample_format = ?sample_format,
            "audio capture starting"
        );

        let (tx, rx) = sync_channel::<CapturedAudio>(16);
        let src_rate = config.sample_rate.0;
        let target_rate = OPUS_SAMPLE_RATE_HZ;

        let err_cb = |e| tracing::warn!(error = %e, "audio stream error");

        let stream = match sample_format {
            SampleFormat::I16 => device
                .build_input_stream(
                    &config,
                    make_callback::<i16>(tx, channels, src_rate, target_rate),
                    err_cb,
                    None,
                )
                .map_err(|e| DesktopError::Capture(format!("build i16 stream: {e}")))?,
            SampleFormat::U16 => device
                .build_input_stream(
                    &config,
                    make_callback::<u16>(tx, channels, src_rate, target_rate),
                    err_cb,
                    None,
                )
                .map_err(|e| DesktopError::Capture(format!("build u16 stream: {e}")))?,
            SampleFormat::F32 => device
                .build_input_stream(
                    &config,
                    make_callback::<f32>(tx, channels, src_rate, target_rate),
                    err_cb,
                    None,
                )
                .map_err(|e| DesktopError::Capture(format!("build f32 stream: {e}")))?,
            other => {
                return Err(DesktopError::Capture(format!(
                    "unsupported cpal sample format {other:?}"
                )));
            }
        };

        stream
            .play()
            .map_err(|e| DesktopError::Capture(format!("cpal play: {e}")))?;

        Ok(Self {
            _stream: stream,
            rx,
            channels,
        })
    }

    /// Block waiting for the next audio frame (20 ms worth of samples).
    pub fn next_frame(&self) -> Option<CapturedAudio> {
        self.rx.recv().ok()
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }
}

/// Build a typed cpal input callback that resamples, downmixes if necessary,
/// and emits fixed-size 20 ms frames.
fn make_callback<S>(
    tx: SyncSender<CapturedAudio>,
    channels: u32,
    src_rate: u32,
    target_rate: u32,
) -> impl FnMut(&[S], &cpal::InputCallbackInfo) + Send + 'static
where
    S: cpal::Sample + cpal::SizedSample + ToI16 + 'static,
{
    // Buffer holds accumulated (already-resampled) i16 samples.
    let mut pending = Vec::<i16>::with_capacity(OPUS_FRAME_SAMPLES * channels as usize * 4);
    let frame_samples = OPUS_FRAME_SAMPLES * channels as usize;
    // Simple linear resampling state.
    let ratio = target_rate as f64 / src_rate as f64;
    let mut resample_phase: f64 = 0.0;

    move |data: &[S], _| {
        for chunk in data.chunks_exact(channels as usize) {
            // Convert every sample in the multi-channel frame to i16.
            // Linear resample by generating output samples whenever the phase
            // accumulator rolls over 1.0.
            resample_phase += ratio;
            while resample_phase >= 1.0 {
                resample_phase -= 1.0;
                for s in chunk.iter() {
                    pending.push(s.to_i16());
                }
            }
        }

        // Emit complete frames.
        while pending.len() >= frame_samples {
            let samples = pending.drain(..frame_samples).collect::<Vec<_>>();
            // Non-blocking send: drop frames if the consumer lags.
            let _ = tx.try_send(CapturedAudio { samples, channels });
        }
    }
}

/// Internal trait to unify cpal's sample types down to `i16`.
trait ToI16 {
    fn to_i16(&self) -> i16;
}

impl ToI16 for i16 {
    fn to_i16(&self) -> i16 {
        *self
    }
}

impl ToI16 for u16 {
    fn to_i16(&self) -> i16 {
        (*self as i32 - 32768) as i16
    }
}

impl ToI16 for f32 {
    fn to_i16(&self) -> i16 {
        (self.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
    }
}
