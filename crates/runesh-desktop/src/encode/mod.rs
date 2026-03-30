//! Frame encoding: compress captured frames for efficient transport.

use crate::capture::CapturedFrame;
use crate::error::DesktopError;
use crate::protocol::{Encoding, Quality};

/// Encoded frame ready for transport.
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub encoding: Encoding,
    pub width: u32,
    pub height: u32,
    pub is_key_frame: bool,
}

/// Encode a frame based on the requested quality and encoding.
pub fn encode_frame(
    frame: &CapturedFrame,
    quality: Quality,
    encoding: Encoding,
) -> Result<EncodedFrame, DesktopError> {
    match encoding {
        Encoding::Raw => encode_raw(frame),
        Encoding::Zstd => encode_zstd(frame, quality),
        Encoding::Png => encode_png(frame),
        Encoding::Jpeg => encode_jpeg(frame, quality),
    }
}

/// Raw encoding — no compression, just pass through.
fn encode_raw(frame: &CapturedFrame) -> Result<EncodedFrame, DesktopError> {
    Ok(EncodedFrame {
        data: frame.data.clone(),
        encoding: Encoding::Raw,
        width: frame.width,
        height: frame.height,
        is_key_frame: true,
    })
}

/// Zstd compression of raw pixel data — fast and good compression.
fn encode_zstd(frame: &CapturedFrame, quality: Quality) -> Result<EncodedFrame, DesktopError> {
    let level = match quality {
        Quality::Low => 1,
        Quality::Medium => 3,
        Quality::High => 6,
        Quality::Lossless => 9,
    };

    let compressed = zstd::encode_all(frame.data.as_slice(), level)
        .map_err(|e| DesktopError::Encoding(format!("Zstd encoding failed: {e}")))?;

    Ok(EncodedFrame {
        data: compressed,
        encoding: Encoding::Zstd,
        width: frame.width,
        height: frame.height,
        is_key_frame: true,
    })
}

/// PNG encoding — lossless but slower.
fn encode_png(frame: &CapturedFrame) -> Result<EncodedFrame, DesktopError> {
    // Convert BGRA to RGBA for PNG
    let mut rgba_data = frame.data.clone();
    for chunk in rgba_data.chunks_exact_mut(4) {
        chunk.swap(0, 2); // B <-> R
    }

    let mut png_data = Vec::new();
    {
        let encoder =
            image::codecs::png::PngEncoder::new_with_quality(
                &mut png_data,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Sub,
            );

        use image::ImageEncoder;
        encoder
            .write_image(&rgba_data, frame.width, frame.height, image::ExtendedColorType::Rgba8)
            .map_err(|e| DesktopError::Encoding(format!("PNG encoding failed: {e}")))?;
    }

    Ok(EncodedFrame {
        data: png_data,
        encoding: Encoding::Png,
        width: frame.width,
        height: frame.height,
        is_key_frame: true,
    })
}

/// JPEG encoding — lossy but very fast and small.
fn encode_jpeg(frame: &CapturedFrame, quality: Quality) -> Result<EncodedFrame, DesktopError> {
    let jpeg_quality = match quality {
        Quality::Low => 30,
        Quality::Medium => 60,
        Quality::High => 85,
        Quality::Lossless => 100,
    };

    // Convert BGRA to RGB for JPEG (no alpha channel)
    let mut rgb_data = Vec::with_capacity((frame.width * frame.height * 3) as usize);
    for chunk in frame.data.chunks_exact(4) {
        rgb_data.push(chunk[2]); // R
        rgb_data.push(chunk[1]); // G
        rgb_data.push(chunk[0]); // B
    }

    let mut jpeg_data = Vec::new();
    {
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg_data,
            jpeg_quality,
        );

        use image::ImageEncoder;
        encoder
            .write_image(&rgb_data, frame.width, frame.height, image::ExtendedColorType::Rgb8)
            .map_err(|e| DesktopError::Encoding(format!("JPEG encoding failed: {e}")))?;
    }

    Ok(EncodedFrame {
        data: jpeg_data,
        encoding: Encoding::Jpeg,
        width: frame.width,
        height: frame.height,
        is_key_frame: true,
    })
}

/// Choose the best encoding based on quality setting.
pub fn auto_encoding(quality: Quality) -> Encoding {
    match quality {
        Quality::Low => Encoding::Jpeg,
        Quality::Medium => Encoding::Jpeg,
        Quality::High => Encoding::Zstd,
        Quality::Lossless => Encoding::Png,
    }
}
