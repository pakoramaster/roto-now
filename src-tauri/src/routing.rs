use crate::models::ModelId;
use image::{imageops::FilterType, DynamicImage};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityMode {
    Fast,
    Balanced,
    Maximum,
}

impl QualityMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.eq_ignore_ascii_case("fast") {
            Ok(Self::Fast)
        } else if value.eq_ignore_ascii_case("balanced") {
            Ok(Self::Balanced)
        } else if value.eq_ignore_ascii_case("maximum") {
            Ok(Self::Maximum)
        } else {
            Err("Quality must be Fast, Balanced, or Maximum".into())
        }
    }

    pub fn general_model(self) -> ModelId {
        if self == Self::Maximum {
            ModelId::General
        } else {
            ModelId::GeneralLite
        }
    }
}

pub fn select_model(
    requested: &str,
    quality: QualityMode,
    sample: Option<&DynamicImage>,
    anime_installed: bool,
) -> Result<ModelId, String> {
    if requested.eq_ignore_ascii_case("anime") {
        return Ok(ModelId::Anime);
    }
    if requested.eq_ignore_ascii_case("general") {
        return Ok(quality.general_model());
    }
    if !requested.eq_ignore_ascii_case("auto") {
        return Err("Detection model must be Auto, General, or Anime".into());
    }
    if anime_installed && sample.is_some_and(looks_stylized) {
        Ok(ModelId::Anime)
    } else {
        Ok(quality.general_model())
    }
}

pub fn looks_stylized(source: &DynamicImage) -> bool {
    let image = source
        .resize_exact(128, 128, FilterType::Triangle)
        .to_rgb8();
    let mut palette = HashSet::new();
    for pixel in image.pixels() {
        palette.insert(
            ((pixel[0] >> 4) as u16) << 8 | ((pixel[1] >> 4) as u16) << 4 | (pixel[2] >> 4) as u16,
        );
    }

    let mut comparisons = 0_u64;
    let mut flat = 0_u64;
    let mut edges = 0_u64;
    for y in 0..image.height() {
        for x in 0..image.width() {
            let current = image.get_pixel(x, y);
            if x > 0 {
                measure_pair(
                    current.0,
                    image.get_pixel(x - 1, y).0,
                    &mut comparisons,
                    &mut flat,
                    &mut edges,
                );
            }
            if y > 0 {
                measure_pair(
                    current.0,
                    image.get_pixel(x, y - 1).0,
                    &mut comparisons,
                    &mut flat,
                    &mut edges,
                );
            }
        }
    }
    let comparisons = comparisons.max(1) as f32;
    let flat_ratio = flat as f32 / comparisons;
    let edge_ratio = edges as f32 / comparisons;
    let palette_ratio = palette.len() as f32 / (image.width() * image.height()) as f32;
    flat_ratio >= 0.58 && (0.025..=0.36).contains(&edge_ratio) && palette_ratio <= 0.12
}

fn measure_pair(
    first: [u8; 3],
    second: [u8; 3],
    comparisons: &mut u64,
    flat: &mut u64,
    edges: &mut u64,
) {
    *comparisons += 1;
    let color_delta = (u8::abs_diff(first[0], second[0]) as u16
        + u8::abs_diff(first[1], second[1]) as u16
        + u8::abs_diff(first[2], second[2]) as u16)
        / 3;
    if color_delta <= 8 {
        *flat += 1;
    }
    let first_luma = first[0] as i32 * 54 + first[1] as i32 * 183 + first[2] as i32 * 19;
    let second_luma = second[0] as i32 * 54 + second[1] as i32 * 183 + second[2] as i32 * 19;
    if (first_luma - second_luma).unsigned_abs() >= 45 * 256 {
        *edges += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn maximum_uses_the_large_general_model() {
        assert_eq!(QualityMode::Maximum.general_model(), ModelId::General);
        assert_eq!(QualityMode::Fast.general_model(), ModelId::GeneralLite);
    }

    #[test]
    fn graphic_line_art_is_detected_conservatively() {
        let image = ImageBuffer::from_fn(128, 128, |x, y| {
            if x % 12 == 0 || y % 12 == 0 {
                Rgb([8, 8, 12])
            } else {
                Rgb([235, 190, 80])
            }
        });
        assert!(looks_stylized(&DynamicImage::ImageRgb8(image)));
    }

    #[test]
    fn textured_content_stays_on_the_general_route() {
        let image = ImageBuffer::from_fn(128, 128, |x, y| {
            Rgb([
                ((x * 37 + y * 17) % 256) as u8,
                ((x * 11 + y * 53) % 256) as u8,
                ((x * 71 + y * 7) % 256) as u8,
            ])
        });
        assert!(!looks_stylized(&DynamicImage::ImageRgb8(image)));
    }

    #[test]
    fn auto_only_routes_to_anime_when_it_is_installed() {
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_fn(128, 128, |x, y| {
            if x % 12 == 0 || y % 12 == 0 {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 180, 60])
            }
        }));
        assert_eq!(
            select_model("Auto", QualityMode::Balanced, Some(&image), true).unwrap(),
            ModelId::Anime
        );
        assert_eq!(
            select_model("Auto", QualityMode::Balanced, Some(&image), false).unwrap(),
            ModelId::GeneralLite
        );
    }
}
