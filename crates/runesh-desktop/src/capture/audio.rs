//! Cross-platform audio capture, producing 20 ms frames at 48 kHz `i16`
//! that match [`crate::encode::opus_enc`]'s expected input.
//!
//! ## Backends
//!
//! - **Windows**: cpal's WASAPI backend opened on the default **output**
//!   device, which enables loopback capture of the host's audio output.
//! - **macOS**: [`screencapturekit`] `SCStream` with `capturesAudio=true`.
//!   This is the only sanctioned way to get real system-audio loopback on
//!   macOS 13+ without an aggregate device or a kext. The video output of
//!   the stream is intentionally minimized and ignored.
//! - **Linux**: cpal's default input device. On PulseAudio/PipeWire, users
//!   route a monitor source as the default input to capture host audio.

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
    #[cfg(target_os = "macos")]
    inner: macos_audio::MacAudioCapturer,
    #[cfg(not(target_os = "macos"))]
    inner: cpal_audio::CpalAudioCapturer,
}

impl AudioCapturer {
    pub fn start() -> Result<Self, DesktopError> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                inner: macos_audio::MacAudioCapturer::start()?,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Self {
                inner: cpal_audio::CpalAudioCapturer::start()?,
            })
        }
    }

    /// Block waiting for the next audio frame (20 ms worth of samples).
    pub fn next_frame(&self) -> Option<CapturedAudio> {
        self.inner.next_frame()
    }

    pub fn channels(&self) -> u32 {
        self.inner.channels()
    }
}

// ── macOS: ScreenCaptureKit system-audio loopback ─────────────────────────

#[cfg(target_os = "macos")]
mod macos_audio {
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use screencapturekit::prelude::*;

    use super::{CapturedAudio, OPUS_FRAME_SAMPLES, OPUS_SAMPLE_RATE_HZ};
    use crate::error::DesktopError;

    /// ScreenCaptureKit delivers audio as 32-bit float in native-endian, typically
    /// deinterleaved across channels (one buffer per channel). We normalise to
    /// interleaved `i16` to match the other backends.
    struct AudioHandler {
        tx: SyncSender<CapturedAudio>,
        channels: u32,
        pending: Mutex<Vec<i16>>,
    }

    impl SCStreamOutputTrait for AudioHandler {
        fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
            if of_type != SCStreamOutputType::Audio {
                return;
            }
            let Some(list) = sample.audio_buffer_list() else {
                return;
            };
            let ch = self.channels as usize;
            let num_buffers = list.num_buffers();
            if num_buffers == 0 || ch == 0 {
                return;
            }
            let Some(first) = list.get(0) else {
                return;
            };
            let samples_per_channel = first.data_byte_size() / std::mem::size_of::<f32>();
            if samples_per_channel == 0 {
                return;
            }

            // SCStream is usually non-interleaved: num_buffers == channels.
            // If the OS happens to deliver interleaved (num_buffers == 1), the
            // same code path works — we just read from a single buffer.
            let non_interleaved = num_buffers >= ch;
            let mut interleaved: Vec<i16> = Vec::with_capacity(samples_per_channel * ch);
            for i in 0..samples_per_channel {
                for c in 0..ch {
                    let (buf, idx_in_buf) = if non_interleaved {
                        let bi = c.min(num_buffers - 1);
                        let Some(b) = list.get(bi) else {
                            continue;
                        };
                        (b, i)
                    } else {
                        (first, i * ch + c)
                    };
                    let data = buf.data();
                    let offset = idx_in_buf * std::mem::size_of::<f32>();
                    if offset + 4 > data.len() {
                        continue;
                    }
                    let f = f32::from_le_bytes([
                        data[offset],
                        data[offset + 1],
                        data[offset + 2],
                        data[offset + 3],
                    ]);
                    interleaved.push((f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                }
            }

            // Chunk into exact 20 ms Opus frames.
            let Ok(mut pending) = self.pending.lock() else {
                return;
            };
            pending.extend_from_slice(&interleaved);
            let frame_samples = OPUS_FRAME_SAMPLES * ch;
            while pending.len() >= frame_samples {
                let samples: Vec<i16> = pending.drain(..frame_samples).collect();
                let _ = self.tx.try_send(CapturedAudio {
                    samples,
                    channels: ch as u32,
                });
            }
        }
    }

    /// Holds the `SCStream` so that dropping the capturer stops capture.
    pub(crate) struct MacAudioCapturer {
        stream: SCStream,
        rx: Receiver<CapturedAudio>,
        channels: u32,
    }

    impl MacAudioCapturer {
        pub(crate) fn start() -> Result<Self, DesktopError> {
            let content = SCShareableContent::get()
                .map_err(|e| DesktopError::Capture(format!("SCShareableContent: {e}")))?;
            let display = content
                .displays()
                .into_iter()
                .next()
                .ok_or_else(|| DesktopError::Capture("no displays available".into()))?;

            // We only want audio. The stream still produces video frames for the
            // chosen display — shrink them to 2x2 and never attach a handler for
            // `SCStreamOutputType::Screen`, so the samples are dropped by the
            // framework.
            let filter = SCContentFilter::create()
                .with_display(&display)
                .with_excluding_windows(&[])
                .build();

            let channels: u32 = 2;
            let config = SCStreamConfiguration::new()
                .with_width(2)
                .with_height(2)
                .with_captures_audio(true)
                .with_sample_rate(OPUS_SAMPLE_RATE_HZ as _)
                .with_channel_count(channels as _);

            let (tx, rx) = sync_channel::<CapturedAudio>(16);
            let handler = AudioHandler {
                tx,
                channels,
                pending: Mutex::new(Vec::new()),
            };

            let mut stream = SCStream::new(&filter, &config);
            stream.add_output_handler(handler, SCStreamOutputType::Audio);
            stream
                .start_capture()
                .map_err(|e| DesktopError::Capture(format!("SCStream start: {e}")))?;

            tracing::info!(
                channels,
                sample_rate = OPUS_SAMPLE_RATE_HZ,
                "macOS ScreenCaptureKit system-audio capture running"
            );

            Ok(Self {
                stream,
                rx,
                channels,
            })
        }

        pub(crate) fn next_frame(&self) -> Option<CapturedAudio> {
            self.rx.recv().ok()
        }
        pub(crate) fn channels(&self) -> u32 {
            self.channels
        }
    }

    impl Drop for MacAudioCapturer {
        fn drop(&mut self) {
            if let Err(e) = self.stream.stop_capture() {
                tracing::warn!(error = %e, "SCStream stop_capture failed");
            }
        }
    }
}

// ── Windows (WASAPI loopback) + Linux (default input via cpal) ────────────

#[cfg(not(target_os = "macos"))]
mod cpal_audio {
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, StreamConfig};

    use super::{CapturedAudio, OPUS_FRAME_SAMPLES, OPUS_SAMPLE_RATE_HZ};
    use crate::error::DesktopError;

    pub(crate) struct CpalAudioCapturer {
        _stream: cpal::Stream,
        rx: Receiver<CapturedAudio>,
        channels: u32,
    }

    impl CpalAudioCapturer {
        pub(crate) fn start() -> Result<Self, DesktopError> {
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
                sample_rate = config.sample_rate,
                channels,
                sample_format = ?sample_format,
                "audio capture starting"
            );

            let (tx, rx) = sync_channel::<CapturedAudio>(16);
            let src_rate = config.sample_rate;
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

        pub(crate) fn next_frame(&self) -> Option<CapturedAudio> {
            self.rx.recv().ok()
        }
        pub(crate) fn channels(&self) -> u32 {
            self.channels
        }
    }

    fn make_callback<S>(
        tx: SyncSender<CapturedAudio>,
        channels: u32,
        src_rate: u32,
        target_rate: u32,
    ) -> impl FnMut(&[S], &cpal::InputCallbackInfo) + Send + 'static
    where
        S: cpal::Sample + cpal::SizedSample + ToI16 + 'static,
    {
        let mut pending = Vec::<i16>::with_capacity(OPUS_FRAME_SAMPLES * channels as usize * 4);
        let frame_samples = OPUS_FRAME_SAMPLES * channels as usize;
        let ratio = target_rate as f64 / src_rate as f64;
        let mut resample_phase: f64 = 0.0;

        move |data: &[S], _| {
            for chunk in data.chunks_exact(channels as usize) {
                resample_phase += ratio;
                while resample_phase >= 1.0 {
                    resample_phase -= 1.0;
                    for s in chunk.iter() {
                        pending.push(s.to_i16());
                    }
                }
            }

            while pending.len() >= frame_samples {
                let samples = pending.drain(..frame_samples).collect::<Vec<_>>();
                let _ = tx.try_send(CapturedAudio { samples, channels });
            }
        }
    }

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
}
