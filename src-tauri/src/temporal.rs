use image::DynamicImage;

const SCENE_CUT_THRESHOLD: f32 = 0.18;
const MOTION_RANGE: f32 = 64.0;
const HISTORY_STRENGTH: f32 = 0.58;
const DROPOUT_RATIO: f32 = 0.08;
const MIN_VISIBLE_ALPHA: u64 = 4;
const MAX_DROPOUT_HOLD_FRAMES: u8 = 2;

#[derive(Default)]
pub struct TemporalMaskStabilizer {
    previous_rgb: Vec<u8>,
    previous_alpha: Vec<u8>,
    current_alpha: Vec<u8>,
    previous_alpha_sum: u64,
    dropout_hold_frames: u8,
}

impl TemporalMaskStabilizer {
    pub fn apply_and_composite(
        &mut self,
        rgb: &[u8],
        cutout: &mut DynamicImage,
        screen_color: &str,
        output: &mut Vec<u8>,
    ) -> Result<(), String> {
        let rgba = cutout
            .as_mut_rgba8()
            .ok_or("Temporal stabilization requires an RGBA cutout")?;
        let pixel_count = rgba.width() as usize * rgba.height() as usize;
        if rgb.len() != pixel_count * 3 {
            return Err("Temporal stabilization received a mismatched video frame".into());
        }

        self.current_alpha.clear();
        self.current_alpha
            .extend(rgba.as_raw().chunks_exact(4).map(|pixel| pixel[3]));
        let current_alpha_sum = self.current_alpha.iter().map(|value| *value as u64).sum();
        let delta = frame_delta(&self.previous_rgb, rgb);
        let reset = self.previous_rgb.len() != rgb.len()
            || self.previous_alpha.len() != pixel_count
            || delta >= SCENE_CUT_THRESHOLD;

        if reset {
            self.previous_rgb.clear();
            self.previous_rgb.extend_from_slice(rgb);
            self.previous_alpha.clone_from(&self.current_alpha);
            self.previous_alpha_sum = current_alpha_sum;
            self.dropout_hold_frames = 0;
            composite_rgba(rgba.as_raw(), screen_color, output);
            return Ok(());
        }

        // A healthy mask can occasionally collapse to almost zero for one or
        // two otherwise continuous frames. Holding the last stable mask avoids
        // a full green/blue flash. The short limit still lets a subject leave
        // the frame naturally and scene cuts always bypass the hold.
        let minimum_previous_sum = pixel_count as u64 * MIN_VISIBLE_ALPHA;
        let mask_dropped_out = self.previous_alpha_sum >= minimum_previous_sum
            && (current_alpha_sum as f32) < self.previous_alpha_sum as f32 * DROPOUT_RATIO
            && self.dropout_hold_frames < MAX_DROPOUT_HOLD_FRAMES;
        if mask_dropped_out {
            self.dropout_hold_frames += 1;
            for (pixel, previous) in rgba
                .as_mut()
                .chunks_exact_mut(4)
                .zip(self.previous_alpha.iter())
            {
                pixel[3] = *previous;
            }
            self.previous_rgb.copy_from_slice(rgb);
            composite_rgba(rgba.as_raw(), screen_color, output);
            return Ok(());
        }
        self.dropout_hold_frames = 0;

        let mut stabilized_alpha_sum = 0_u64;
        output.clear();
        output.reserve(pixel_count * 3);
        let background = screen_background(screen_color);
        for (index, pixel) in rgba.as_mut().chunks_exact_mut(4).enumerate() {
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
            stabilized_alpha_sum += pixel[3] as u64;
            composite_pixel(pixel, background, output);
        }
        self.previous_alpha_sum = stabilized_alpha_sum;
        self.previous_rgb.copy_from_slice(rgb);
        Ok(())
    }
}

fn screen_background(screen_color: &str) -> [u8; 3] {
    if screen_color == "blue" {
        [0, 71, 187]
    } else {
        [0, 177, 64]
    }
}

fn composite_pixel(pixel: &[u8], background: [u8; 3], output: &mut Vec<u8>) {
    let alpha = pixel[3] as u16;
    let inverse = 255 - alpha;
    output.extend_from_slice(&[
        ((pixel[0] as u16 * alpha + background[0] as u16 * inverse + 127) / 255) as u8,
        ((pixel[1] as u16 * alpha + background[1] as u16 * inverse + 127) / 255) as u8,
        ((pixel[2] as u16 * alpha + background[2] as u16 * inverse + 127) / 255) as u8,
    ]);
}

fn composite_rgba(rgba: &[u8], screen_color: &str, output: &mut Vec<u8>) {
    output.clear();
    output.reserve(rgba.len() / 4 * 3);
    let background = screen_background(screen_color);
    for pixel in rgba.chunks_exact(4) {
        composite_pixel(pixel, background, output);
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

    fn apply(stabilizer: &mut TemporalMaskStabilizer, rgb: &[u8], frame: &mut DynamicImage) {
        stabilizer
            .apply_and_composite(rgb, frame, "green", &mut Vec::new())
            .unwrap();
    }

    #[test]
    fn first_frame_is_not_modified() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut frame = cutout(120, 1);
        apply(&mut stabilizer, &[20, 30, 40], &mut frame);
        assert_eq!(alpha(&frame, 0), 120);
    }

    #[test]
    fn stable_pixels_smooth_mask_jitter() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let rgb = [20, 30, 40].repeat(8);
        let mut first = cutout(100, 8);
        apply(&mut stabilizer, &rgb, &mut first);
        let mut second = cutout(140, 8);
        apply(&mut stabilizer, &rgb, &mut second);
        assert!(alpha(&second, 0) > 100);
        assert!(alpha(&second, 0) < 140);
    }

    #[test]
    fn scene_cut_resets_mask_history() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut first = cutout(20, 8);
        apply(&mut stabilizer, &vec![0; 24], &mut first);
        let mut second = cutout(230, 8);
        apply(&mut stabilizer, &vec![255; 24], &mut second);
        assert_eq!(alpha(&second, 0), 230);
    }

    #[test]
    fn moving_pixels_follow_the_current_mask() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let pixels = 100;
        let first_rgb = vec![0; pixels * 3];
        let mut first = cutout(20, pixels as u32);
        apply(&mut stabilizer, &first_rgb, &mut first);

        let mut second_rgb = first_rgb;
        second_rgb[0..3].fill(255);
        let mut second = cutout(230, pixels as u32);
        apply(&mut stabilizer, &second_rgb, &mut second);
        assert_eq!(alpha(&second, 0), 230);
        assert!(alpha(&second, 1) < 230);
    }

    #[test]
    fn short_empty_mask_dropouts_reuse_the_last_stable_mask() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let rgb = [20, 30, 40].repeat(16);
        let mut stable = cutout(220, 16);
        apply(&mut stabilizer, &rgb, &mut stable);

        for _ in 0..MAX_DROPOUT_HOLD_FRAMES {
            let mut dropout = cutout(0, 16);
            apply(&mut stabilizer, &rgb, &mut dropout);
            assert_eq!(alpha(&dropout, 0), 220);
        }

        let mut sustained_empty = cutout(0, 16);
        apply(&mut stabilizer, &rgb, &mut sustained_empty);
        assert!(alpha(&sustained_empty, 0) < 220);
    }

    #[test]
    fn compositing_uses_the_selected_screen_color() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut frame = cutout(0, 1);
        let mut output = Vec::new();
        stabilizer
            .apply_and_composite(&[20, 30, 40], &mut frame, "blue", &mut output)
            .unwrap();
        assert_eq!(output, vec![0, 71, 187]);
    }
}
