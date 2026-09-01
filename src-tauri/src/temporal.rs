use image::DynamicImage;

const SCENE_CUT_THRESHOLD: f32 = 0.18;
const MOTION_RANGE: f32 = 64.0;
const HISTORY_STRENGTH: f32 = 0.58;
const OUTLIER_MINIMUM_DELTA: u8 = 16;
const OUTLIER_CORRECTION_STRENGTH: f32 = 0.90;
const DROPOUT_RATIO: f32 = 0.08;
const MIN_VISIBLE_ALPHA: u64 = 4;
const MAX_DROPOUT_HOLD_FRAMES: u8 = 2;

#[derive(Default)]
pub struct TemporalMaskStabilizer {
    previous_alpha: Vec<u8>,
    pending_rgb: Vec<u8>,
    pending_alpha: Vec<u8>,
    pending_similarity: Vec<u8>,
    current_alpha: Vec<u8>,
    previous_alpha_sum: u64,
    dropout_hold_frames: u8,
    pending_reset_before: bool,
}

impl TemporalMaskStabilizer {
    pub fn push_frame(
        &mut self,
        rgb: &[u8],
        cutout: &DynamicImage,
        screen_color: &str,
        output: &mut Vec<u8>,
    ) -> Result<bool, String> {
        let rgba = cutout
            .as_rgba8()
            .ok_or("Temporal stabilization requires an RGBA cutout")?;
        let pixel_count = rgba.width() as usize * rgba.height() as usize;
        if rgb.len() != pixel_count * 3 {
            return Err("Temporal stabilization received a mismatched video frame".into());
        }

        self.current_alpha.clear();
        self.current_alpha
            .extend(rgba.as_raw().chunks_exact(4).map(|pixel| pixel[3]));
        output.clear();

        if self.pending_alpha.is_empty() {
            self.pending_rgb.clear();
            self.pending_rgb.extend_from_slice(rgb);
            std::mem::swap(&mut self.pending_alpha, &mut self.current_alpha);
            self.pending_reset_before = true;
            return Ok(false);
        }
        if self.pending_rgb.len() != rgb.len() || self.pending_alpha.len() != pixel_count {
            return Err("Temporal stabilization received a mismatched video frame".into());
        }

        let reset_after = frame_delta(&self.pending_rgb, rgb) >= SCENE_CUT_THRESHOLD;
        let next_alpha = std::mem::take(&mut self.current_alpha);
        self.render_pending(
            Some(rgb),
            Some(&next_alpha),
            reset_after,
            screen_color,
            output,
        );

        transition_similarity(
            &self.pending_rgb,
            rgb,
            reset_after,
            &mut self.pending_similarity,
        );
        self.pending_rgb.clear();
        self.pending_rgb.extend_from_slice(rgb);
        let recycled_alpha = std::mem::replace(&mut self.pending_alpha, next_alpha);
        self.current_alpha = recycled_alpha;
        self.current_alpha.clear();
        self.pending_reset_before = reset_after;
        Ok(true)
    }

    pub fn finish(&mut self, screen_color: &str, output: &mut Vec<u8>) -> Result<bool, String> {
        output.clear();
        if self.pending_alpha.is_empty() {
            return Ok(false);
        }
        self.render_pending(None, None, true, screen_color, output);
        self.pending_rgb.clear();
        self.pending_alpha.clear();
        self.pending_similarity.clear();
        self.pending_reset_before = true;
        Ok(true)
    }

    fn render_pending(
        &mut self,
        next_rgb: Option<&[u8]>,
        next_alpha: Option<&[u8]>,
        reset_after: bool,
        screen_color: &str,
        output: &mut Vec<u8>,
    ) {
        let pixel_count = self.pending_alpha.len();
        let pending_alpha_sum: u64 = self.pending_alpha.iter().map(|value| *value as u64).sum();
        let has_history = !self.previous_alpha.is_empty() && !self.pending_reset_before;
        let minimum_previous_sum = pixel_count as u64 * MIN_VISIBLE_ALPHA;
        let mask_dropped_out = has_history
            && self.previous_alpha_sum >= minimum_previous_sum
            && (pending_alpha_sum as f32) < self.previous_alpha_sum as f32 * DROPOUT_RATIO
            && self.dropout_hold_frames < MAX_DROPOUT_HOLD_FRAMES;

        if mask_dropped_out {
            self.dropout_hold_frames += 1;
            self.pending_alpha.copy_from_slice(&self.previous_alpha);
        } else if has_history {
            self.dropout_hold_frames = 0;
            for index in 0..pixel_count {
                let raw = self.pending_alpha[index];
                let previous = self.previous_alpha[index];
                let before_similarity = self.pending_similarity[index] as f32 / 255.0;
                let after_similarity = if reset_after {
                    0.0
                } else {
                    next_rgb
                        .map(|next| pixel_similarity(&self.pending_rgb, next, index))
                        .unwrap_or(0.0)
                };
                let corrected = match next_alpha {
                    Some(next) if !reset_after => {
                        let future = next[index];
                        let median = median3(previous, raw, future);
                        let is_outlier = u8::abs_diff(raw, median) >= OUTLIER_MINIMUM_DELTA;
                        if is_outlier {
                            let stability = before_similarity.min(after_similarity);
                            let weight = OUTLIER_CORRECTION_STRENGTH * stability * stability;
                            blend(raw, median, weight)
                        } else {
                            raw
                        }
                    }
                    _ => raw,
                };
                let alpha_delta = u8::abs_diff(corrected, previous) as f32 / 255.0;
                let agreement = 1.0 - alpha_delta * 0.55;
                let history_weight =
                    HISTORY_STRENGTH * before_similarity * before_similarity * agreement;
                self.pending_alpha[index] = blend(corrected, previous, history_weight);
            }
        } else {
            self.dropout_hold_frames = 0;
        }

        composite_rgb_alpha(&self.pending_rgb, &self.pending_alpha, screen_color, output);
        self.previous_alpha.clear();
        self.previous_alpha.extend_from_slice(&self.pending_alpha);
        self.previous_alpha_sum = self.previous_alpha.iter().map(|value| *value as u64).sum();
    }
}

fn transition_similarity(previous: &[u8], current: &[u8], reset: bool, output: &mut Vec<u8>) {
    let pixel_count = current.len() / 3;
    output.clear();
    output.resize(pixel_count, 0);
    if reset {
        return;
    }
    for (index, similarity) in output.iter_mut().enumerate() {
        *similarity = (pixel_similarity(previous, current, index) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
}

fn pixel_similarity(previous: &[u8], current: &[u8], pixel: usize) -> f32 {
    let index = pixel * 3;
    let color_delta = (u8::abs_diff(previous[index], current[index]) as f32
        + u8::abs_diff(previous[index + 1], current[index + 1]) as f32
        + u8::abs_diff(previous[index + 2], current[index + 2]) as f32)
        / 3.0;
    (1.0 - color_delta / MOTION_RANGE).clamp(0.0, 1.0)
}

fn median3(first: u8, second: u8, third: u8) -> u8 {
    first
        .min(second)
        .max(first.min(third).min(second.max(third)))
}

fn blend(current: u8, target: u8, weight: f32) -> u8 {
    (current as f32 * (1.0 - weight) + target as f32 * weight)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn screen_background(screen_color: &str) -> [u8; 3] {
    if screen_color == "blue" {
        [0, 71, 187]
    } else {
        [0, 177, 64]
    }
}

pub fn composite_rgb_alpha(rgb: &[u8], alpha: &[u8], screen_color: &str, output: &mut Vec<u8>) {
    output.clear();
    output.reserve(rgb.len());
    let background = screen_background(screen_color);
    for (pixel, alpha) in rgb.chunks_exact(3).zip(alpha.iter()) {
        let alpha = *alpha as u16;
        let inverse = 255 - alpha;
        output.extend_from_slice(&[
            ((pixel[0] as u16 * alpha + background[0] as u16 * inverse + 127) / 255) as u8,
            ((pixel[1] as u16 * alpha + background[1] as u16 * inverse + 127) / 255) as u8,
            ((pixel[2] as u16 * alpha + background[2] as u16 * inverse + 127) / 255) as u8,
        ]);
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

    fn frame(alpha: u8, pixels: u32, color: [u8; 3]) -> (Vec<u8>, DynamicImage) {
        let rgb = color.repeat(pixels as usize);
        let cutout = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            pixels,
            1,
            Rgba([color[0], color[1], color[2], alpha]),
        ));
        (rgb, cutout)
    }

    fn push(
        stabilizer: &mut TemporalMaskStabilizer,
        alpha: u8,
        pixels: u32,
        color: [u8; 3],
        output: &mut Vec<u8>,
    ) -> bool {
        let (rgb, cutout) = frame(alpha, pixels, color);
        stabilizer
            .push_frame(&rgb, &cutout, "green", output)
            .unwrap()
    }

    #[test]
    fn first_frame_waits_for_lookahead_and_flushes() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        assert!(!push(&mut stabilizer, 255, 1, [200, 10, 20], &mut output));
        assert!(output.is_empty());
        assert!(stabilizer.finish("green", &mut output).unwrap());
        assert_eq!(output, vec![200, 10, 20]);
        assert!(!stabilizer.finish("green", &mut output).unwrap());
    }

    #[test]
    fn two_frames_are_emitted_in_order() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        assert!(!push(&mut stabilizer, 255, 1, [200, 10, 20], &mut output));
        assert!(push(&mut stabilizer, 255, 1, [10, 20, 200], &mut output));
        assert_eq!(output, vec![200, 10, 20]);
        assert!(stabilizer.finish("green", &mut output).unwrap());
        assert_eq!(output, vec![10, 20, 200]);
    }

    #[test]
    fn stable_pixels_smooth_mask_jitter() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        push(&mut stabilizer, 100, 8, [20, 30, 40], &mut output);
        push(&mut stabilizer, 140, 8, [20, 30, 40], &mut output);
        push(&mut stabilizer, 140, 8, [20, 30, 40], &mut output);
        assert!(stabilizer.previous_alpha[0] > 100);
        assert!(stabilizer.previous_alpha[0] < 140);
    }

    #[test]
    fn isolated_mask_hole_is_corrected() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        push(&mut stabilizer, 220, 16, [20, 30, 40], &mut output);
        push(&mut stabilizer, 0, 16, [20, 30, 40], &mut output);
        push(&mut stabilizer, 220, 16, [20, 30, 40], &mut output);
        assert_eq!(stabilizer.previous_alpha[0], 220);
    }

    #[test]
    fn isolated_foreground_pop_is_corrected() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        push(&mut stabilizer, 0, 16, [20, 30, 40], &mut output);
        push(&mut stabilizer, 220, 16, [20, 30, 40], &mut output);
        push(&mut stabilizer, 0, 16, [20, 30, 40], &mut output);
        assert!(stabilizer.previous_alpha[0] < 32);
    }

    #[test]
    fn genuine_motion_follows_the_pending_mask() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        push(&mut stabilizer, 0, 16, [0, 0, 0], &mut output);
        push(&mut stabilizer, 220, 16, [255, 255, 255], &mut output);
        push(&mut stabilizer, 0, 16, [0, 0, 0], &mut output);
        assert_eq!(stabilizer.previous_alpha[0], 220);
    }

    #[test]
    fn scene_cut_resets_mask_history() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        push(&mut stabilizer, 20, 16, [0, 0, 0], &mut output);
        push(&mut stabilizer, 230, 16, [255, 255, 255], &mut output);
        push(&mut stabilizer, 230, 16, [255, 255, 255], &mut output);
        assert_eq!(stabilizer.previous_alpha[0], 230);
    }

    #[test]
    fn two_frame_dropout_hold_does_not_block_sustained_disappearance() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        push(&mut stabilizer, 220, 16, [20, 30, 40], &mut output);
        push(&mut stabilizer, 0, 16, [20, 30, 40], &mut output);
        push(&mut stabilizer, 0, 16, [20, 30, 40], &mut output);
        assert_eq!(stabilizer.previous_alpha[0], 220);
        push(&mut stabilizer, 0, 16, [20, 30, 40], &mut output);
        assert_eq!(stabilizer.previous_alpha[0], 220);
        push(&mut stabilizer, 0, 16, [20, 30, 40], &mut output);
        assert!(stabilizer.previous_alpha[0] < 220);
    }

    #[test]
    fn compositing_uses_the_selected_screen_color() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        let (rgb, cutout) = frame(0, 1, [20, 30, 40]);
        assert!(!stabilizer
            .push_frame(&rgb, &cutout, "blue", &mut output)
            .unwrap());
        assert!(stabilizer.finish("blue", &mut output).unwrap());
        assert_eq!(output, vec![0, 71, 187]);
    }

    #[test]
    fn mismatched_dimensions_are_rejected() {
        let mut stabilizer = TemporalMaskStabilizer::default();
        let mut output = Vec::new();
        push(&mut stabilizer, 120, 1, [20, 30, 40], &mut output);
        let (rgb, cutout) = frame(120, 2, [20, 30, 40]);
        assert!(stabilizer
            .push_frame(&rgb, &cutout, "green", &mut output)
            .is_err());
    }
}
