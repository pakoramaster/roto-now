use crate::{
    jobs::JobControl,
    models::{model_path, ModelId},
    routing::QualityMode,
};
use image::{imageops::FilterType, DynamicImage, GenericImageView, GrayImage, ImageBuffer, Luma};
use ort::{
    ep,
    session::{RunOptions, Session},
    value::Tensor,
};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::AppHandle;

const INPUT_SIZE: u32 = 1024;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

pub struct Masker {
    session: Session,
    model_id: ModelId,
    provider: &'static str,
    model_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelFingerprint {
    path: PathBuf,
    length: u64,
    modified: Option<std::time::SystemTime>,
}

impl ModelFingerprint {
    fn read(path: PathBuf) -> Result<Self, String> {
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("Could not inspect model {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("Model path is not a file: {}", path.display()));
        }
        Ok(Self {
            path,
            length: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

struct CachedMasker {
    fingerprint: ModelFingerprint,
    masker: Masker,
}

/// Keeps native ONNX sessions alive between jobs.
///
/// Roto Now permits only one foreground job at a time, so holding this lock
/// through inference also provides the mutable access required by ONNX Runtime.
#[derive(Default)]
pub struct ModelSessionCache {
    entries: Mutex<HashMap<ModelId, CachedMasker>>,
}

impl ModelSessionCache {
    pub fn with_model<R>(
        &self,
        path: PathBuf,
        model_id: ModelId,
        prefer_directml: bool,
        on_load: impl FnOnce(),
        operation: impl FnOnce(&mut Masker, bool) -> Result<R, String>,
    ) -> Result<R, String> {
        let fingerprint = ModelFingerprint::read(path.clone())?;
        let mut entries = self.entries.lock();
        let reused = entries
            .get(&model_id)
            .is_some_and(|entry| entry.fingerprint == fingerprint);

        if !reused {
            // Drop a stale session before loading its replacement to avoid a
            // large transient memory spike and to release its model file.
            entries.remove(&model_id);
            on_load();
            let masker = Masker::load_from_path(path, model_id, prefer_directml)?;
            entries.insert(
                model_id,
                CachedMasker {
                    fingerprint,
                    masker,
                },
            );
        }

        let entry = entries
            .get_mut(&model_id)
            .ok_or("The segmentation model cache was not initialized")?;
        operation(&mut entry.masker, reused)
    }

    pub fn invalidate(&self, model_id: ModelId) {
        self.entries.lock().remove(&model_id);
    }
}

impl Masker {
    pub fn load(app: &AppHandle, model_id: ModelId, prefer_directml: bool) -> Result<Self, String> {
        let path = model_path(app, model_id)?;
        Self::load_from_path(path, model_id, prefer_directml)
    }

    pub fn load_from_path(
        path: PathBuf,
        model_id: ModelId,
        prefer_directml: bool,
    ) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!(
                "{} is not installed",
                crate::models::spec(model_id).name
            ));
        }

        if prefer_directml {
            let directml = (|| {
                let builder = Session::builder().map_err(|error| error.to_string())?;
                let builder = builder
                    .with_parallel_execution(false)
                    .map_err(|error| error.to_string())?;
                let builder = builder
                    .with_memory_pattern(false)
                    .map_err(|error| error.to_string())?;
                let mut builder = builder
                    .with_execution_providers([ep::DirectML::default().build()])
                    .map_err(|error| error.to_string())?;
                builder
                    .commit_from_file(&path)
                    .map_err(|error| error.to_string())
            })();
            if let Ok(session) = directml {
                return Ok(Self {
                    session,
                    model_id,
                    provider: "DmlExecutionProvider",
                    model_path: path,
                });
            }
        }

        let session = (|| {
            let builder = Session::builder().map_err(|error| error.to_string())?;
            let builder = builder
                .with_parallel_execution(false)
                .map_err(|error| error.to_string())?;
            let mut builder = builder
                .with_memory_pattern(false)
                .map_err(|error| error.to_string())?;
            builder
                .commit_from_file(&path)
                .map_err(|error| error.to_string())
        })()
        .map_err(|error| {
            format!(
                "Could not load {}: {error}",
                crate::models::spec(model_id).name
            )
        })?;
        Ok(Self {
            session,
            model_id,
            provider: "CPUExecutionProvider",
            model_path: path,
        })
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    pub fn apply(
        &mut self,
        source: &DynamicImage,
        edge_detail: u8,
        quality: &str,
        control: &JobControl,
    ) -> Result<DynamicImage, String> {
        let (width, height) = source.dimensions();
        let source_rgb = source.to_rgb8();
        let quality = QualityMode::parse(quality)?;
        let resize_filter = if quality == QualityMode::Fast {
            FilterType::Triangle
        } else {
            FilterType::Lanczos3
        };
        let rgb = image::imageops::resize(&source_rgb, INPUT_SIZE, INPUT_SIZE, resize_filter);
        let divisor = if self.model_id == ModelId::Anime {
            255.0
        } else {
            rgb.as_raw().iter().copied().max().unwrap_or(1).max(1) as f32
        };
        let plane = (INPUT_SIZE * INPUT_SIZE) as usize;
        let mut input = vec![0.0_f32; plane * 3];
        for (index, pixel) in rgb.pixels().enumerate() {
            for channel in 0..3 {
                input[channel * plane + index] =
                    (pixel[channel] as f32 / divisor - MEAN[channel]) / STD[channel];
            }
        }

        if control.cancelled.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let mut mask = match run_session(&mut self.session, &input, control) {
            Ok(mask) => mask,
            Err(error) if error != "cancelled" && self.provider == "DmlExecutionProvider" => {
                let builder = Session::builder().map_err(|error| error.to_string())?;
                let builder = builder
                    .with_parallel_execution(false)
                    .map_err(|error| error.to_string())?;
                let mut builder = builder
                    .with_memory_pattern(false)
                    .map_err(|error| error.to_string())?;
                self.session = builder
                    .commit_from_file(&self.model_path)
                    .map_err(|cpu_error| {
                        format!("DirectML failed ({error}); CPU fallback also failed: {cpu_error}")
                    })?;
                self.provider = "CPUExecutionProvider";
                run_session(&mut self.session, &input, control)?
            }
            Err(error) => return Err(error),
        };

        if mask.len() < plane {
            return Err("Model returned an undersized mask".into());
        }
        mask.truncate(plane);
        if self.model_id != ModelId::Anime {
            for value in &mut mask {
                *value = 1.0 / (1.0 + (-*value).exp());
            }
            let minimum = mask.iter().copied().fold(f32::INFINITY, f32::min);
            let maximum = mask.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let range = (maximum - minimum).max(f32::EPSILON);
            for value in &mut mask {
                *value = (*value - minimum) / range;
            }
        } else if mask.iter().any(|value| *value < -1e-4 || *value > 1.0001) {
            for value in &mut mask {
                *value = 1.0 / (1.0 + (-*value).exp());
            }
        }

        let raw: Vec<u8> = mask
            .into_iter()
            .map(|value| (value.clamp(0.0, 1.0) * 255.0) as u8)
            .collect();
        let small = GrayImage::from_raw(INPUT_SIZE, INPUT_SIZE, raw)
            .ok_or("Could not construct output mask")?;
        let resized = image::imageops::resize(&small, width, height, resize_filter);
        let alpha = refine_alpha(&resized, edge_detail);
        let mut rgba = source.to_rgba8();
        for (pixel, alpha) in rgba.pixels_mut().zip(alpha.pixels()) {
            pixel.0[3] = alpha.0[0];
        }
        Ok(DynamicImage::ImageRgba8(rgba))
    }
}

fn run_session(
    session: &mut Session,
    input: &[f32],
    control: &JobControl,
) -> Result<Vec<f32>, String> {
    let tensor = Tensor::from_array((
        [1_usize, 3, INPUT_SIZE as usize, INPUT_SIZE as usize],
        input.to_vec().into_boxed_slice(),
    ))
    .map_err(|error| format!("Could not prepare inference input: {error}"))?;
    let run_options = Arc::new(
        RunOptions::new()
            .map_err(|error| format!("Could not create inference controls: {error}"))?,
    );
    let watcher_done = Arc::new(AtomicBool::new(false));
    let watcher = {
        let options = Arc::clone(&run_options);
        let cancelled = Arc::clone(&control.cancelled);
        let done = Arc::clone(&watcher_done);
        thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                if cancelled.load(Ordering::SeqCst) {
                    let _ = options.terminate();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let output_result = session.run_with_options(ort::inputs![tensor], &run_options);
    watcher_done.store(true, Ordering::Release);
    let _ = watcher.join();
    let outputs = output_result.map_err(|error| {
        if control.cancelled.load(Ordering::SeqCst) {
            "cancelled".into()
        } else {
            format!("Inference failed: {error}")
        }
    })?;
    let (_, values) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|error| format!("Model returned an invalid mask: {error}"))?;
    Ok(values.to_vec())
}

fn refine_alpha(alpha: &GrayImage, edge_detail: u8) -> GrayImage {
    let strength = 0.72 + (edge_detail.min(100) as f32 / 100.0) * 0.72;
    ImageBuffer::from_fn(alpha.width(), alpha.height(), |x, y| {
        let original = alpha.get_pixel(x, y).0[0];
        let refined = if original <= 2 {
            0
        } else if original >= 253 {
            255
        } else {
            let normalized = (original as f32 / 255.0).clamp(1e-4, 1.0 - 1e-4);
            let logit = (normalized / (1.0 - normalized)).ln();
            ((1.0 / (1.0 + (-logit * strength).exp())) * 255.0).clamp(0.0, 255.0) as u8
        };
        Luma([refined])
    })
}

pub fn save_cutout(image: &DynamicImage, path: &Path) -> Result<(), String> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("Could not save PNG: {error}"))
}

pub fn composite_screen(cutout: &DynamicImage, screen_color: &str) -> Vec<u8> {
    let background = if screen_color == "blue" {
        [0, 71, 187]
    } else {
        [0, 177, 64]
    };
    cutout
        .to_rgba8()
        .pixels()
        .flat_map(|pixel| {
            let alpha = pixel[3] as f32 / 255.0;
            [
                (pixel[0] as f32 * alpha + background[0] as f32 * (1.0 - alpha)) as u8,
                (pixel[1] as f32 * alpha + background[1] as f32 * (1.0 - alpha)) as u8,
                (pixel[2] as f32 * alpha + background[2] as f32 * (1.0 - alpha)) as u8,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn alpha_refinement_preserves_hard_limits() {
        let alpha = GrayImage::from_raw(4, 1, vec![0, 2, 253, 255]).unwrap();
        assert_eq!(refine_alpha(&alpha, 72).into_raw(), vec![0, 0, 255, 255]);
    }

    #[test]
    fn model_fingerprint_changes_when_file_is_replaced() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roto-now-model-cache-{suffix}.onnx"));
        fs::write(&path, b"first").unwrap();
        let first = ModelFingerprint::read(path.clone()).unwrap();
        fs::write(&path, b"replacement").unwrap();
        let replacement = ModelFingerprint::read(path.clone()).unwrap();
        let _ = fs::remove_file(path);

        assert_ne!(first, replacement);
    }
}
