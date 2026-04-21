//! Software H.264 encoder backed by Cisco's OpenH264.
//!
//! BGRA frames from the capturer are converted to YUV 4:2:0 and fed into the
//! encoder. The output is an Annex-B NAL unit byte stream suitable for the
//! WebRTC `TrackLocalStaticSample` and for file dumps.

use std::time::Duration;

use openh264::OpenH264API;
use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, RateControlMode,
};
use openh264::formats::{BgraSliceU8, YUVBuffer};

use super::{VideoEncoder, VideoSample};
use crate::capture::CapturedFrame;
use crate::error::DesktopError;
use crate::protocol::Quality;

pub struct OpenH264Encoder {
    inner: Encoder,
    width: u32,
    height: u32,
    fps: u32,
    force_idr: bool,
    frame_index: u64,
    bitrate_kbps: u32,
}

impl OpenH264Encoder {
    pub fn new(width: u32, height: u32, quality: Quality, fps: u32) -> Result<Self, DesktopError> {
        if width == 0 || height == 0 {
            return Err(DesktopError::Encoding(
                "zero-size frame rejected by encoder".into(),
            ));
        }
        // OpenH264 wants even dimensions for 4:2:0 chroma subsampling.
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(DesktopError::Encoding(format!(
                "H.264 requires even frame dimensions, got {width}x{height}"
            )));
        }

        let bitrate_kbps = quality.target_kbps_for(width, height);
        let fps = fps.clamp(1, 120);

        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_kbps * 1000))
            .rate_control_mode(RateControlMode::Bitrate)
            .max_frame_rate(FrameRate::from_hz(fps as f32))
            // Generate an IDR every 2 seconds so late-joining viewers can sync.
            .intra_frame_period(IntraFramePeriod::from_num_frames(fps.saturating_mul(2)));

        let inner = Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|e| DesktopError::Encoding(format!("openh264 init: {e}")))?;

        tracing::info!(
            width,
            height,
            fps,
            bitrate_kbps,
            codec = "openh264-software",
            "H.264 encoder initialised"
        );

        Ok(Self {
            inner,
            width,
            height,
            fps,
            force_idr: true, // first frame is always an IDR
            frame_index: 0,
            bitrate_kbps,
        })
    }

    fn encode_yuv(&mut self, frame: &CapturedFrame) -> Result<(Vec<u8>, bool), DesktopError> {
        if frame.width != self.width || frame.height != self.height {
            return Err(DesktopError::Encoding(format!(
                "frame dimensions {}x{} do not match encoder {}x{}",
                frame.width, frame.height, self.width, self.height
            )));
        }
        let expected = (self.width as usize) * (self.height as usize) * 4;
        if frame.data.len() < expected {
            return Err(DesktopError::Encoding(format!(
                "BGRA buffer too small: {} < {}",
                frame.data.len(),
                expected
            )));
        }

        let bgra = BgraSliceU8::new(
            &frame.data[..expected],
            (self.width as usize, self.height as usize),
        );
        let yuv = YUVBuffer::from_rgb_source(bgra);

        if self.force_idr {
            self.inner.force_intra_frame();
            self.force_idr = false;
        }

        let bitstream = self
            .inner
            .encode(&yuv)
            .map_err(|e| DesktopError::Encoding(format!("openh264 encode: {e}")))?;

        // Gather NAL units into an Annex-B byte stream and note whether an
        // IDR slice is present.
        let mut out = Vec::with_capacity(bitstream.num_layers().saturating_mul(64));
        let mut is_keyframe = false;
        for layer_idx in 0..bitstream.num_layers() {
            let Some(layer) = bitstream.layer(layer_idx) else {
                continue;
            };
            for nal_idx in 0..layer.nal_count() {
                let Some(nal) = layer.nal_unit(nal_idx) else {
                    continue;
                };
                if nal.is_empty() {
                    continue;
                }
                // Annex-B: NAL units are already prefixed with start codes by
                // the openh264 writer. Copy verbatim.
                out.extend_from_slice(nal);
                // Extract the NAL header (byte 0 after any leading start code).
                if let Some(&header) = skip_start_code(nal).first() {
                    let nal_type = header & 0x1F;
                    if nal_type == 5 {
                        // 5 = IDR slice
                        is_keyframe = true;
                    }
                }
            }
        }

        Ok((out, is_keyframe))
    }
}

fn skip_start_code(nal: &[u8]) -> &[u8] {
    if nal.starts_with(&[0, 0, 0, 1]) {
        &nal[4..]
    } else if nal.starts_with(&[0, 0, 1]) {
        &nal[3..]
    } else {
        nal
    }
}

impl VideoEncoder for OpenH264Encoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Option<VideoSample>, DesktopError> {
        let (data, is_keyframe) = self.encode_yuv(frame)?;
        if data.is_empty() {
            return Ok(None);
        }
        self.frame_index = self.frame_index.wrapping_add(1);
        Ok(Some(VideoSample {
            data,
            duration: Duration::from_secs(1) / self.fps.max(1),
            is_keyframe,
        }))
    }

    fn force_keyframe(&mut self) {
        self.force_idr = true;
    }

    fn set_bitrate_kbps(&mut self, kbps: u32) {
        // OpenH264's Rust binding does not expose runtime bitrate mutation.
        // The encoder's internal rate control still reacts to scene complexity;
        // for big quality changes, tear down and rebuild the encoder.
        self.bitrate_kbps = kbps.clamp(100, 100_000);
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn codec_name(&self) -> &'static str {
        "openh264-software"
    }
}
