//! Frame encoding.
//!
//! [`VideoEncoder`] is the WebRTC-first codec interface. Encodes BGRA frames
//! into H.264 Annex-B byte streams suitable for
//! [`webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample`].
//!
//! The default implementation is [`h264::OpenH264Encoder`] (software encoder,
//! works on all platforms). Hardware backends (NVENC, Media Foundation,
//! VideoToolbox, VA-API) plug in behind the same trait.
//!
//! The audio counterpart lives in [`opus_enc`].

use std::time::Duration;

use crate::capture::CapturedFrame;
use crate::error::DesktopError;
use crate::protocol::Quality;

#[cfg(feature = "h264")]
pub mod h264;

#[cfg(feature = "audio")]
pub mod opus_enc;

/// An encoded video sample ready for [`webrtc::track::track_local::track_local_static_sample::Sample`].
pub struct VideoSample {
    /// H.264 Annex-B NAL units. Start codes (`00 00 00 01`) precede each NALU.
    pub data: Vec<u8>,
    /// Nominal duration between this sample and the next. Used by the RTP
    /// packetizer to compute timestamps.
    pub duration: Duration,
    /// True when this sample contains an IDR frame (decoders can sync here).
    pub is_keyframe: bool,
}

/// Video encoder trait. Implementors must be `Send` to run in a background task.
pub trait VideoEncoder: Send {
    /// Encode a single captured BGRA frame. Returns `Ok(None)` if the encoder
    /// deliberately skipped the frame (e.g. rate control), `Err` on failure.
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Option<VideoSample>, DesktopError>;

    /// Ask the encoder to emit an IDR at the next `encode` call.
    fn force_keyframe(&mut self);

    /// Change the target bitrate in kilobits/second.
    fn set_bitrate_kbps(&mut self, kbps: u32);

    /// Width/height the encoder was initialised with.
    fn dimensions(&self) -> (u32, u32);

    /// Short human-readable name for logs (e.g. `"openh264-software"`).
    fn codec_name(&self) -> &'static str;
}

/// Create the default video encoder for the given resolution and quality.
///
/// The `h264` feature must be enabled; otherwise this returns
/// [`DesktopError::Unsupported`].
pub fn create_video_encoder(
    width: u32,
    height: u32,
    quality: Quality,
    fps: u32,
) -> Result<Box<dyn VideoEncoder>, DesktopError> {
    #[cfg(feature = "h264")]
    {
        let enc = h264::OpenH264Encoder::new(width, height, quality, fps)?;
        Ok(Box::new(enc))
    }
    #[cfg(not(feature = "h264"))]
    {
        let _ = (width, height, quality, fps);
        Err(DesktopError::Unsupported(
            "no video encoder compiled in (enable the `h264` feature)".into(),
        ))
    }
}
