use image::RgbaImage;
use serde::Deserialize;

const MAX_STROKES: usize = 500;
const MAX_POINTS: usize = 20_000;
const MAX_STAMPS: usize = 2_000_000;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionStroke {
    pub mode: String,
    pub radius: f32,
    pub points: Vec<CorrectionPoint>,
}

pub fn apply_corrections(
    image: &mut RgbaImage,
    strokes: &[CorrectionStroke],
) -> Result<(), String> {
    if strokes.is_empty() {
        return Err("Draw at least one correction before applying".into());
    }
    if strokes.len() > MAX_STROKES
        || strokes
            .iter()
            .map(|stroke| stroke.points.len())
            .sum::<usize>()
            > MAX_POINTS
    {
        return Err("The correction contains too many brush points".into());
    }

    let mut stamps = 0_usize;
    for stroke in strokes {
        apply_stroke(image, stroke, &mut stamps)?;
    }
    Ok(())
}

fn apply_stroke(
    image: &mut RgbaImage,
    stroke: &CorrectionStroke,
    stamps: &mut usize,
) -> Result<(), String> {
    let restore = match stroke.mode.as_str() {
        "restore" => true,
        "erase" => false,
        _ => return Err("Correction mode must be restore or erase".into()),
    };
    if !stroke.radius.is_finite() || !(0.002..=0.25).contains(&stroke.radius) {
        return Err("Correction brush size is outside the supported range".into());
    }
    if stroke.points.is_empty() {
        return Ok(());
    }
    for point in &stroke.points {
        if !point.x.is_finite()
            || !point.y.is_finite()
            || !(0.0..=1.0).contains(&point.x)
            || !(0.0..=1.0).contains(&point.y)
        {
            return Err("Correction points must stay inside the image".into());
        }
    }

    let radius = (stroke.radius * image.width().min(image.height()) as f32).max(1.0);
    let mut previous = &stroke.points[0];
    stamp(image, previous, radius, restore);
    *stamps += 1;
    for point in stroke.points.iter().skip(1) {
        let dx = (point.x - previous.x) * image.width() as f32;
        let dy = (point.y - previous.y) * image.height() as f32;
        let steps = (dx.hypot(dy) / (radius * 0.35)).ceil().max(1.0) as usize;
        *stamps = stamps.saturating_add(steps);
        if *stamps > MAX_STAMPS {
            return Err("The correction is too complex to apply safely".into());
        }
        for step in 1..=steps {
            let amount = step as f32 / steps as f32;
            let sample = CorrectionPoint {
                x: previous.x + (point.x - previous.x) * amount,
                y: previous.y + (point.y - previous.y) * amount,
            };
            stamp(image, &sample, radius, restore);
        }
        previous = point;
    }
    Ok(())
}

fn stamp(image: &mut RgbaImage, point: &CorrectionPoint, radius: f32, restore: bool) {
    let center_x = point.x * (image.width().saturating_sub(1)) as f32;
    let center_y = point.y * (image.height().saturating_sub(1)) as f32;
    let min_x = (center_x - radius).floor().max(0.0) as u32;
    let max_x = (center_x + radius)
        .ceil()
        .min(image.width().saturating_sub(1) as f32) as u32;
    let min_y = (center_y - radius).floor().max(0.0) as u32;
    let max_y = (center_y + radius)
        .ceil()
        .min(image.height().saturating_sub(1) as f32) as u32;
    let solid_radius = radius * 0.72;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let distance = (x as f32 - center_x).hypot(y as f32 - center_y);
            if distance > radius {
                continue;
            }
            let coverage = if distance <= solid_radius {
                1.0
            } else {
                1.0 - (distance - solid_radius) / (radius - solid_radius).max(f32::EPSILON)
            };
            let alpha = &mut image.get_pixel_mut(x, y).0[3];
            if restore {
                *alpha = (*alpha).max((coverage * 255.0).round() as u8);
            } else {
                *alpha = (*alpha).min(((1.0 - coverage) * 255.0).round() as u8);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn stroke(mode: &str) -> CorrectionStroke {
        CorrectionStroke {
            mode: mode.into(),
            radius: 0.2,
            points: vec![CorrectionPoint { x: 0.5, y: 0.5 }],
        }
    }

    #[test]
    fn restore_brush_makes_the_center_opaque() {
        let mut image = ImageBuffer::from_pixel(20, 20, Rgba([10, 20, 30, 0]));
        apply_corrections(&mut image, &[stroke("restore")]).unwrap();
        assert_eq!(image.get_pixel(10, 10)[3], 255);
        assert_eq!(image.get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn erase_brush_makes_the_center_transparent() {
        let mut image = ImageBuffer::from_pixel(20, 20, Rgba([10, 20, 30, 255]));
        apply_corrections(&mut image, &[stroke("erase")]).unwrap();
        assert_eq!(image.get_pixel(10, 10)[3], 0);
        assert_eq!(image.get_pixel(0, 0)[3], 255);
    }

    #[test]
    fn invalid_points_are_rejected() {
        let mut image = ImageBuffer::from_pixel(20, 20, Rgba([10, 20, 30, 255]));
        let mut invalid = stroke("erase");
        invalid.points[0].x = 1.1;
        assert!(apply_corrections(&mut image, &[invalid]).is_err());
    }
}
