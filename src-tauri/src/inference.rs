use crate::{
    jobs::JobControl,
    models::{model_path, ModelId},
    routing::QualityMode,
};
use image::{imageops::FilterType, DynamicImage, GrayImage};
#[cfg(test)]
use image::{ImageBuffer, Luma};
use ort::{
    ep,
    session::{RunOptions, Session},
    value::TensorRef,
};
use parking_lot::Mutex;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::AppHandle;

const INPUT_SIZE: u32 = 1024;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

fn model_precision(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if name.contains("fp16") {
        "FP16"
    } else {
        "FP32"
    }
}

pub struct Masker {
    session: Option<Session>,
    model_id: ModelId,
    provider: &'static str,
    precision: &'static str,
    model_path: PathBuf,
    fallback_model_path: PathBuf,
    input_scratch: Vec<f32>,
    mask_scratch: Vec<u8>,
    rgba_scratch: Vec<u8>,
    last_timing: InferenceTiming,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InferenceTiming {
    pub preprocess: Duration,
    pub inference: Duration,
    pub postprocess: Duration,
}

pub struct PreparedFrame {
    source_rgb: image::RgbImage,
    input: Vec<f32>,
    resize_filter: FilterType,
    preprocess: Duration,
}

impl PreparedFrame {
    pub fn source_bytes(&self) -> &[u8] {
        self.source_rgb.as_raw()
    }

    pub fn into_recycling_parts(self) -> (Vec<u8>, Vec<f32>) {
        (self.source_rgb.into_raw(), self.input)
    }
}

pub fn prepare_video_frame(
    source_rgb: image::RgbImage,
    model_id: ModelId,
    quality: &str,
    input: Vec<f32>,
) -> Result<PreparedFrame, String> {
    prepare_rgb_frame(source_rgb, model_id, quality, input)
}

fn prepare_rgb_frame(
    source_rgb: image::RgbImage,
    model_id: ModelId,
    quality: &str,
    mut input: Vec<f32>,
) -> Result<PreparedFrame, String> {
    let started = Instant::now();
    let resize_filter = match QualityMode::parse(quality)? {
        QualityMode::Fast => FilterType::Triangle,
        QualityMode::Balanced => FilterType::CatmullRom,
        QualityMode::Maximum => FilterType::Lanczos3,
    };
    let rgb = image::imageops::resize(&source_rgb, INPUT_SIZE, INPUT_SIZE, resize_filter);
    let divisor = if model_id == ModelId::Anime {
        255.0
    } else {
        rgb.as_raw().iter().copied().max().unwrap_or(1).max(1) as f32
    };
    let plane = (INPUT_SIZE * INPUT_SIZE) as usize;
    input.resize(plane * 3, 0.0);
    for (index, pixel) in rgb.pixels().enumerate() {
        for channel in 0..3 {
            input[channel * plane + index] =
                (pixel[channel] as f32 / divisor - MEAN[channel]) / STD[channel];
        }
    }
    Ok(PreparedFrame {
        source_rgb,
        input,
        resize_filter,
        preprocess: started.elapsed(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelFingerprint {
    path: PathBuf,
    length: u64,
    modified: Option<std::time::SystemTime>,
    accelerated: Option<(u64, Option<std::time::SystemTime>)>,
}

impl ModelFingerprint {
    fn read(path: PathBuf, model_id: ModelId) -> Result<Self, String> {
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("Could not inspect model {}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("Model path is not a file: {}", path.display()));
        }
        let accelerated = crate::models::accelerated_model_path(&path, model_id)
            .and_then(|path| fs::metadata(path).ok())
            .map(|metadata| (metadata.len(), metadata.modified().ok()));
        Ok(Self {
            path,
            length: metadata.len(),
            modified: metadata.modified().ok(),
            accelerated,
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
    active: Mutex<Option<CachedMasker>>,
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
        let fingerprint = ModelFingerprint::read(path.clone(), model_id)?;
        let mut active = self.active.lock();
        let reused = active.as_ref().is_some_and(|entry| {
            entry.masker.model_id == model_id && entry.fingerprint == fingerprint
        });

        if !reused {
            // Retain only the active model. DirectML sessions can reserve most
            // of an 8 GB GPU, so keeping sessions for previously used models
            // makes later GPU allocations much more likely to fail.
            *active = None;
            on_load();
            let masker = Masker::load_from_path(path, model_id, prefer_directml)?;
            *active = Some(CachedMasker {
                fingerprint,
                masker,
            });
        }

        let entry = active
            .as_mut()
            .ok_or("The segmentation model cache was not initialized")?;
        operation(&mut entry.masker, reused)
    }

    pub fn invalidate(&self, model_id: ModelId) {
        let mut active = self.active.lock();
        if active
            .as_ref()
            .is_some_and(|entry| entry.masker.model_id == model_id)
        {
            *active = None;
        }
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

        let accelerated_path = (std::env::var_os("ROTO_NOW_DISABLE_FP16").as_deref()
            != Some(std::ffi::OsStr::new("1")))
        .then(|| crate::models::accelerated_model_path(&path, model_id))
        .flatten();
        if prefer_directml {
            let dml_path = accelerated_path.as_ref().unwrap_or(&path);
            let directml = (|| {
                let builder = Session::builder().map_err(|error| error.to_string())?;
                let builder = builder
                    .with_parallel_execution(false)
                    .map_err(|error| error.to_string())?;
                let builder = builder
                    .with_memory_pattern(false)
                    .map_err(|error| error.to_string())?;
                let mut builder = builder
                    .with_execution_providers([ep::DirectML::default()
                        .with_performance_preference(
                            ep::directml::PerformancePreference::HighPerformance,
                        )
                        .build()])
                    .map_err(|error| error.to_string())?;
                builder
                    .commit_from_file(dml_path)
                    .map_err(|error| error.to_string())
            })();
            if let Ok(session) = directml {
                return Ok(Self {
                    session: Some(session),
                    model_id,
                    provider: "DmlExecutionProvider",
                    precision: model_precision(dml_path),
                    model_path: dml_path.clone(),
                    fallback_model_path: path,
                    input_scratch: Vec::new(),
                    mask_scratch: Vec::new(),
                    rgba_scratch: Vec::new(),
                    last_timing: InferenceTiming::default(),
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
            session: Some(session),
            model_id,
            provider: "CPUExecutionProvider",
            precision: model_precision(&path),
            model_path: path.clone(),
            fallback_model_path: path,
            input_scratch: Vec::new(),
            mask_scratch: Vec::new(),
            rgba_scratch: Vec::new(),
            last_timing: InferenceTiming::default(),
        })
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    pub fn precision(&self) -> &'static str {
        self.precision
    }

    pub fn model_id(&self) -> ModelId {
        self.model_id
    }

    pub fn last_timing(&self) -> InferenceTiming {
        self.last_timing
    }

    pub fn recycle_cutout(&mut self, cutout: DynamicImage) {
        if let DynamicImage::ImageRgba8(image) = cutout {
            self.rgba_scratch = image.into_raw();
        }
    }

    pub fn apply(
        &mut self,
        source: &DynamicImage,
        edge_detail: u8,
        quality: &str,
        control: &JobControl,
    ) -> Result<DynamicImage, String> {
        let mut prepared = prepare_rgb_frame(
            source.to_rgb8(),
            self.model_id,
            quality,
            std::mem::take(&mut self.input_scratch),
        )?;
        let result = self.apply_prepared(&prepared, edge_detail, control);
        self.input_scratch = std::mem::take(&mut prepared.input);
        result
    }

    pub fn apply_prepared(
        &mut self,
        prepared: &PreparedFrame,
        edge_detail: u8,
        control: &JobControl,
    ) -> Result<DynamicImage, String> {
        let (width, height) = prepared.source_rgb.dimensions();
        let source_rgb = &prepared.source_rgb;
        let resize_filter = prepared.resize_filter;
        let plane = (INPUT_SIZE * INPUT_SIZE) as usize;

        if control.cancelled.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let inference_started = Instant::now();
        let mut mask = match run_session(
            self.session
                .as_mut()
                .ok_or("The inference session is unavailable")?,
            &prepared.input,
            control,
        ) {
            Ok(mask) => mask,
            Err(error) if error != "cancelled" && self.provider == "DmlExecutionProvider" => {
                // Release the failed GPU session before allocating the CPU
                // replacement or retry. A concurrently initialized hardware
                // codec can invalidate the D3D device; ONNX Runtime explicitly
                // requires recreating the device in that case.
                self.session.take();
                let directml_retry: Result<(Session, Vec<f32>), String> = (|| {
                    let builder = Session::builder().map_err(|error| error.to_string())?;
                    let builder = builder
                        .with_parallel_execution(false)
                        .map_err(|error| error.to_string())?;
                    let builder = builder
                        .with_memory_pattern(false)
                        .map_err(|error| error.to_string())?;
                    let mut builder = builder
                        .with_execution_providers([ep::DirectML::default()
                            .with_performance_preference(
                                ep::directml::PerformancePreference::HighPerformance,
                            )
                            .build()])
                        .map_err(|error| error.to_string())?;
                    let mut session = builder
                        .commit_from_file(&self.model_path)
                        .map_err(|error| error.to_string())?;
                    let mask = run_session(&mut session, &prepared.input, control)?;
                    Ok((session, mask))
                })();
                if let Ok((session, mask)) = directml_retry {
                    self.session = Some(session);
                    mask
                } else {
                    let retry_error = directml_retry
                        .err()
                        .unwrap_or_else(|| "unknown DirectML retry failure".into());
                    eprintln!(
                        "DirectML inference failed and device recreation did not recover it; switching to CPU: {error}; retry: {retry_error}"
                    );
                    let builder = Session::builder().map_err(|error| error.to_string())?;
                    let builder = builder
                        .with_parallel_execution(false)
                        .map_err(|error| error.to_string())?;
                    let mut builder = builder
                        .with_memory_pattern(false)
                        .map_err(|error| error.to_string())?;
                    self.session = Some(
                        builder
                            .commit_from_file(&self.fallback_model_path)
                            .map_err(|cpu_error| {
                                format!(
                                "DirectML failed ({error}); CPU fallback also failed: {cpu_error}"
                            )
                            })?,
                    );
                    self.provider = "CPUExecutionProvider";
                    self.precision = model_precision(&self.fallback_model_path);
                    self.model_path.clone_from(&self.fallback_model_path);
                    run_session(
                        self.session
                            .as_mut()
                            .ok_or("The inference session is unavailable")?,
                        &prepared.input,
                        control,
                    )?
                }
            }
            Err(error) => return Err(error),
        };
        let inference = inference_started.elapsed();
        let postprocess_started = Instant::now();

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

        self.mask_scratch.clear();
        self.mask_scratch.extend(
            mask.into_iter()
                .map(|value| (value.clamp(0.0, 1.0) * 255.0) as u8),
        );
        let small = GrayImage::from_raw(
            INPUT_SIZE,
            INPUT_SIZE,
            std::mem::take(&mut self.mask_scratch),
        )
        .ok_or("Could not construct output mask")?;
        let resized = image::imageops::resize(&small, width, height, resize_filter);
        self.mask_scratch = small.into_raw();
        self.rgba_scratch
            .resize(width as usize * height as usize * 4, 0);
        for ((output, source), alpha) in self
            .rgba_scratch
            .chunks_exact_mut(4)
            .zip(source_rgb.as_raw().chunks_exact(3))
            .zip(resized.as_raw())
        {
            output[..3].copy_from_slice(source);
            output[3] = refine_alpha_value(*alpha, edge_detail);
        }
        let rgba =
            image::RgbaImage::from_raw(width, height, std::mem::take(&mut self.rgba_scratch))
                .ok_or("Could not construct RGBA cutout")?;
        self.last_timing = InferenceTiming {
            preprocess: prepared.preprocess,
            inference,
            postprocess: postprocess_started.elapsed(),
        };
        Ok(DynamicImage::ImageRgba8(rgba))
    }
}

fn run_session(
    session: &mut Session,
    input: &[f32],
    control: &JobControl,
) -> Result<Vec<f32>, String> {
    // Borrow the reusable CHW buffer directly. Tensor::from_array would copy
    // about 12 MiB for every 1024x1024 RGB inference.
    let tensor = TensorRef::from_array_view((
        [1_usize, 3, INPUT_SIZE as usize, INPUT_SIZE as usize],
        input,
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
                thread::park_timeout(Duration::from_millis(10));
            }
        })
    };
    let output_result = session.run_with_options(ort::inputs![tensor], &run_options);
    watcher_done.store(true, Ordering::Release);
    watcher.thread().unpark();
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

#[cfg(test)]
fn refine_alpha(alpha: &GrayImage, edge_detail: u8) -> GrayImage {
    ImageBuffer::from_fn(alpha.width(), alpha.height(), |x, y| {
        Luma([refine_alpha_value(alpha.get_pixel(x, y).0[0], edge_detail)])
    })
}

fn refine_alpha_value(original: u8, edge_detail: u8) -> u8 {
    if original <= 2 {
        return 0;
    }
    if original >= 253 {
        return 255;
    }
    let strength = 0.72 + (edge_detail.min(100) as f32 / 100.0) * 0.72;
    let normalized = (original as f32 / 255.0).clamp(1e-4, 1.0 - 1e-4);
    let logit = (normalized / (1.0 - normalized)).ln();
    ((1.0 / (1.0 + (-logit * strength).exp())) * 255.0).clamp(0.0, 255.0) as u8
}

pub fn save_cutout(image: &DynamicImage, path: &Path) -> Result<(), String> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("Could not save PNG: {error}"))
}

pub fn composite_screen(cutout: &DynamicImage, screen_color: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(cutout.width() as usize * cutout.height() as usize * 3);
    composite_screen_into(cutout, screen_color, &mut output);
    output
}

pub fn composite_screen_into(cutout: &DynamicImage, screen_color: &str, output: &mut Vec<u8>) {
    let background = if screen_color == "blue" {
        [0, 71, 187]
    } else {
        [0, 177, 64]
    };
    output.clear();
    output.reserve(cutout.width() as usize * cutout.height() as usize * 3);
    for pixel in cutout.as_rgba8().expect("cutouts are RGBA").pixels() {
        let alpha = pixel[3] as f32 / 255.0;
        output.extend_from_slice(&[
            (pixel[0] as f32 * alpha + background[0] as f32 * (1.0 - alpha)) as u8,
            (pixel[1] as f32 * alpha + background[1] as f32 * (1.0 - alpha)) as u8,
            (pixel[2] as f32 * alpha + background[2] as f32 * (1.0 - alpha)) as u8,
        ]);
    }
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
        let first = ModelFingerprint::read(path.clone(), ModelId::GeneralLite).unwrap();
        fs::write(&path, b"replacement").unwrap();
        let replacement = ModelFingerprint::read(path.clone(), ModelId::GeneralLite).unwrap();
        let _ = fs::remove_file(path);

        assert_ne!(first, replacement);
    }
}
