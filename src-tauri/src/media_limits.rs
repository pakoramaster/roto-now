//! Bounds decoded media before full-resolution buffers or previews are created.
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use std::path::Path;

const MAX_IMAGE_PIXELS: u64 = 64_000_000;
const MAX_IMAGE_SIDE: u32 = 32_768;
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_VIDEO_PIXELS: u64 = 7680 * 4320;
const MAX_VIDEO_SIDE: u32 = 16_384;

fn validate_dimensions(
    width: u32,
    height: u32,
    max_pixels: u64,
    max_side: u32,
    label: &str,
) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > max_side
        || height > max_side
        || u64::from(width) * u64::from(height) > max_pixels
    {
        return Err(format!("{label} resolution {width}x{height} exceeds the memory safety limit ({} million pixels, maximum side {max_side}). Resize the source before importing it.", max_pixels / 1_000_000));
    }
    Ok(())
}

pub(crate) fn validate_source_image_dimensions(width: u32, height: u32) -> Result<(), String> {
    validate_dimensions(width, height, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, "Image")
}

fn image_decoder(path: &Path) -> Result<impl ImageDecoder, String> {
    let mut reader = ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| format!("Could not inspect image: {error}"))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|error| format!("Could not read image within memory safety limits: {error}"))?;
    let (width, height) = decoder.dimensions();
    validate_dimensions(width, height, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, "Image")?;
    if decoder.total_bytes() > MAX_DECODE_BYTES {
        return Err("Decoded image exceeds the 512 MiB safety limit. Resize the source before importing it.".into());
    }
    Ok(decoder)
}

pub(crate) fn inspect_image(path: &Path) -> Result<(), String> {
    image_decoder(path).map(|_| ())
}

pub(crate) fn open_image(path: &Path) -> Result<DynamicImage, String> {
    DynamicImage::from_decoder(image_decoder(path)?)
        .map_err(|error| format!("Could not decode image within memory safety limits: {error}"))
}

pub(crate) fn video_frame_bytes(width: u32, height: u32) -> Result<usize, String> {
    validate_dimensions(width, height, MAX_VIDEO_PIXELS, MAX_VIDEO_SIDE, "Video")?;
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or("Video frame size overflow")?;
    usize::try_from(bytes).map_err(|_| "Video frame does not fit addressable memory".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_decode_preserves_png_pixels_and_alpha() {
        let source =
            image::RgbaImage::from_raw(2, 1, vec![20, 30, 40, 0, 50, 60, 70, 255]).unwrap();
        let path =
            std::env::temp_dir().join(format!("roto-now-small-image-{}.png", uuid::Uuid::new_v4()));
        source.save(&path).unwrap();
        inspect_image(&path).unwrap();
        let result = open_image(&path).unwrap().to_rgba8();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result, source);
    }

    #[test]
    fn oversized_png_is_rejected_from_headers_before_pixel_decode() {
        // Valid container headers, intentionally tiny pixel payload: dimension
        // rejection must happen before an attempted full pixel decode.
        let bytes: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 31, 65, 0, 0, 31,
            64, 8, 2, 0, 0, 0, 102, 81, 81, 157, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 96, 96,
            96, 0, 0, 0, 4, 0, 1, 246, 23, 56, 85, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        let path =
            std::env::temp_dir().join(format!("roto-now-resolution-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&path, bytes).unwrap();
        let inspection = inspect_image(&path).unwrap_err();
        let decoding = open_image(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(inspection.contains("resolution"), "{inspection}");
        assert!(decoding.contains("resolution"), "{decoding}");
    }

    #[test]
    fn image_pixel_budget_rejects_compact_but_huge_sources() {
        assert!(validate_dimensions(8000, 8000, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, "Image").is_ok());
        assert!(
            validate_dimensions(8001, 8000, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, "Image").is_err()
        );
        assert!(validate_dimensions(
            u32::MAX,
            u32::MAX,
            MAX_IMAGE_PIXELS,
            MAX_IMAGE_SIDE,
            "Image"
        )
        .is_err());
        assert!(validate_dimensions(0, 100, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE, "Image").is_err());
    }
    #[test]
    fn video_budget_preserves_8k_and_rejects_unbounded_frames() {
        assert_eq!(video_frame_bytes(7680, 4320).unwrap(), 7680 * 4320 * 3);
        assert!(video_frame_bytes(4320, 7680).is_ok());
        assert!(video_frame_bytes(16384, 4096).is_err());
        assert!(video_frame_bytes(u32::MAX, u32::MAX).is_err());
    }
}
