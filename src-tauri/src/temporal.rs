use image::DynamicImage;

const SCENE_CUT_THRESHOLD: f32 = 0.18;
const MOTION_RANGE: f32 = 64.0;
const HISTORY_STRENGTH: f32 = 0.58;

#[derive(Default)]
pub struct TemporalMaskStabilizer {
    previous_rgb: Vec<u8>,
    previous_alpha: Vec<u8>,
    current_alpha: Vec<u8>,
}

impl TemporalMaskStabilizer {
    pub fn apply(&mut self, rgb: &[u8], cutout: &mut DynamicImage) -> Result<(), String> {
        let rgba = cutout
            .as_mut_rgba8()
            .ok_or("Temporal stabilization requires an RGBA cutout")?;
        let pixel_count = rgba.width() as usize * rgba.height() as usize;
        if rgb.len() != pixel_count * 3 {
            return Err("Temporal stabilization received a mismatched video frame".into());
        }

        self.current_alpha.clear();
        self.current_alpha
            .extend(rgba.pixels().map(|pixel| pixel[3]));
        let reset = self.previous_rgb.len() != rgb.len()
            || self.previous_alpha.len() != pixel_count
            || frame_delta(&self.previous_rgb, rgb) >= SCENE_CUT_THRESHOLD;

        if reset {
            self.previous_rgb.clear();
            self.previous_rgb.extend_from_slice(rgb);
            self.previous_alpha.clone_from(&self.current_alpha);
            return Ok(());
        }

        for (index, pixel) in rgba.pixels_mut().enumerate() {
            let rgb_index = index * 3;
            let color_delta = (u8::abs_diff(rgb[rgb_index], self.previous_rgb[rgb_index]) as f32
                + u8::abs_diff(rgb[rgb_index + 1], self.previous_rgb[rgb_index + 1]) as f32
                + u8::abs_diff(rgb[rgb_index + 2], self.previous_rgb[rgb_index + 2]) as f32)
                / 3.0;
            let similarity = (1.0 - color_delta / MOTION_RANGE).clamp(0.0, 1.0);
            let alpha_delta =
                u8::abs_diff(self.current_alpha[index], self.previous_alpha[index]) as f32 / 255.0;
            let agreement = 1.0 - alpha_delta * 0.55;
            let history_weight = HISTORY_STRENGTH * similarity * similarity * agreement;
            let stabilized = self.current_alpha[index] as f32 * (1.0 - history_weight)
                + self.previous_alpha[index] as f32 * history_weight;
            pixel[3] = stabilized.round().clamp(0.0, 255.0) as u8;
            self.previous_alpha[index] = pixel[3];
        }
        self.previous_rgb.copy_from_slice(rgb);
        Ok(())
    }
}

fn frame_delta(previous: &[u8], current: &[u8]) -> f32 {
    if previous.len() != current.len() || current.is_empty() {
        return 1.0;
    }
    let pixel_count = current.len() / 3;
    let stride = (pixel_count / 4096).max(1);
    let mut difference = 0_u64;
    let mut samples = 0_u64;
    for pixel in (0..pixel_count).step_by(stride) {
        let index = pixel * 3;
        difference += u8::abs_diff(previous[index], current[index]) as u64;
        difference += u8::abs_diff(previous[index + 1], current[index + 1]) as u64;
        difference += u8::abs_diff(previous[index + 2], current[index + 2]) as u64;
        samples += 1;
    }
    difference as f32 / (samples.max(1) as f32 * 3.0 * 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn cutout(alpha: u8, pixels: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            pixels,
            1,
            Rgba([20, 30, 40, alpha]),
        ))
    }

    fn alpha(image: &DynamicImage, index: u32) -> u8 {
        image.as_rgba8().unwrap().get_pixel(index, 0)[3]
    }

    #[test]
    fn first_frame_is_not_modified() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut frame = cutout(120, 1);
        stabilizer.apply(&[20, 30, 40], &mut frame).unwrap();
        assert_eq!(alpha(&frame, 0), 120);
    }

    #[test]
    fn stable_pixels_smooth_mask_jitter() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let rgb = [20, 30, 40].repeat(8);
        let mut first = cutout(100, 8);
        stabilizer.apply(&rgb, &mut first).unwrap();
        let mut second = cutout(140, 8);
        stabilizer.apply(&rgb, &mut second).unwrap();
        assert!(alpha(&second, 0) > 100);
        assert!(alpha(&second, 0) < 140);
    }

    #[test]
    fn scene_cut_resets_mask_history() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut first = cutout(20, 8);
        stabilizer.apply(&vec![0; 24], &mut first).unwrap();
        let mut second = cutout(230, 8);
        stabilizer.apply(&vec![255; 24], &mut second).unwrap();
        assert_eq!(alpha(&second, 0), 230);
    }

    #[test]
    fn moving_pixels_follow_the_current_mask() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let pixels = 100;
        let first_rgb = vec![0; pixels * 3];
        let mut first = cutout(20, pixels as u32);
        stabilizer.apply(&first_rgb, &mut first).unwrap();

        let mut second_rgb = first_rgb;
        second_rgb[0..3].fill(255);
        let mut second = cutout(230, pixels as u32);
        stabilizer.apply(&second_rgb, &mut second).unwrap();
        assert_eq!(alpha(&second, 0), 230);
        assert!(alpha(&second, 1) < 230);
    }
}
