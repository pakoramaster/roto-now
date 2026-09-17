use crate::jobs::JobControl;
use image::{imageops::FilterType, GrayImage, ImageBuffer, Luma, RgbImage};
use ort::{ep, session::Session, value::TensorRef};
use parking_lot::Mutex;
use std::{collections::VecDeque, path::PathBuf, sync::atomic::Ordering};

const MEMORY_SLOTS: usize = 6;
const MEMORY_EVERY: usize = 5;

#[derive(Clone, Debug)]
pub struct CutieModelPaths {
    pub encode_key: PathBuf,
    pub encode_value: PathBuf,
    pub memory_readout: PathBuf,
    pub decode: PathBuf,
    pub width: usize,
    pub height: usize,
}

impl CutieModelPaths {
    pub fn all_exist(&self) -> bool {
        [
            &self.encode_key,
            &self.encode_value,
            &self.memory_readout,
            &self.decode,
        ]
        .into_iter()
        .all(|path| path.is_file())
    }
}

#[derive(Clone)]
struct MemoryFrame {
    key: Vec<f32>,
    shrinkage: Vec<f32>,
    value: Vec<f32>,
    valid: Vec<f32>,
}

#[derive(Clone, Copy)]
struct Letterbox {
    source_width: u32,
    source_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct KeyFeatures {
    f8: Vec<f32>,
    f4: Vec<f32>,
    pix_feat: Vec<f32>,
    key: Vec<f32>,
    shrinkage: Vec<f32>,
    selection: Vec<f32>,
}

pub struct CutieTracker {
    encode_key: Session,
    encode_value: Session,
    memory_readout: Session,
    decode: Session,
    provider: &'static str,
    model_width: usize,
    model_height: usize,
    grid_width: usize,
    grid_height: usize,
    grid_plane: usize,
    sensory: Vec<f32>,
    last_mask: Vec<f32>,
    object_memory: Vec<f32>,
    permanent_memory: Option<MemoryFrame>,
    working_memory: VecDeque<MemoryFrame>,
    frame_index: usize,
    last_memory_frame: Option<usize>,
    image_input: Vec<f32>,
    mask_input: Vec<f32>,
}

fn session(path: &PathBuf, directml: bool) -> Result<Session, String> {
    let builder = Session::builder().map_err(|error| error.to_string())?;
    let builder = builder
        .with_parallel_execution(false)
        .map_err(|error| error.to_string())?
        .with_memory_pattern(false)
        .map_err(|error| error.to_string())?;
    let mut builder = if directml {
        builder
            .with_execution_providers([ep::DirectML::default()
                .with_performance_preference(ep::directml::PerformancePreference::HighPerformance)
                .build()])
            .map_err(|error| error.to_string())?
    } else {
        builder
    };
    builder
        .commit_from_file(path)
        .map_err(|error| error.to_string())
}

fn load_sessions(paths: &CutieModelPaths, directml: bool) -> Result<[Session; 4], String> {
    Ok([
        session(&paths.encode_key, directml)?,
        session(&paths.encode_value, directml)?,
        session(&paths.memory_readout, directml)?,
        session(&paths.decode, directml)?,
    ])
}

fn output(outputs: &ort::session::SessionOutputs<'_>, name: &str) -> Result<Vec<f32>, String> {
    let value = outputs
        .get(name)
        .ok_or_else(|| format!("Cutie did not return {name}"))?;
    let (_, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|error| format!("Cutie returned an invalid {name}: {error}"))?;
    Ok(data.to_vec())
}

impl CutieTracker {
    pub fn load(paths: &CutieModelPaths, prefer_directml: bool) -> Result<Self, String> {
        if !paths.all_exist() {
            return Err("Cutie Primary Subject is not installed".into());
        }
        if paths.width == 0 || paths.height == 0 || paths.width % 16 != 0 || paths.height % 16 != 0
        {
            return Err("Cutie model dimensions must be positive multiples of 16".into());
        }
        let (sessions, provider) = if prefer_directml {
            match load_sessions(paths, true) {
                Ok(sessions) => (sessions, "DmlExecutionProvider"),
                Err(_) => (load_sessions(paths, false)?, "CPUExecutionProvider"),
            }
        } else {
            (load_sessions(paths, false)?, "CPUExecutionProvider")
        };
        let [encode_key, encode_value, memory_readout, decode] = sessions;
        let grid_width = paths.width / 16;
        let grid_height = paths.height / 16;
        let grid_plane = grid_width * grid_height;
        Ok(Self {
            encode_key,
            encode_value,
            memory_readout,
            decode,
            provider,
            model_width: paths.width,
            model_height: paths.height,
            grid_width,
            grid_height,
            grid_plane,
            sensory: vec![0.0; 256 * grid_plane],
            last_mask: Vec::new(),
            object_memory: Vec::new(),
            permanent_memory: None,
            working_memory: VecDeque::new(),
            frame_index: 0,
            last_memory_frame: None,
            image_input: Vec::new(),
            mask_input: Vec::new(),
        })
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    pub fn internal_size(&self) -> (usize, usize) {
        (self.model_width, self.model_height)
    }

    pub fn reset(&mut self) {
        self.sensory.fill(0.0);
        self.last_mask.clear();
        self.object_memory.clear();
        self.permanent_memory = None;
        self.working_memory.clear();
        self.frame_index = 0;
        self.last_memory_frame = None;
    }

    pub fn step(
        &mut self,
        frame: &RgbImage,
        seed: Option<&GrayImage>,
        control: &JobControl,
    ) -> Result<GrayImage, String> {
        if control.cancelled.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let letterbox = letterbox(
            frame.width(),
            frame.height(),
            self.model_width,
            self.model_height,
        );
        prepare_image(
            frame,
            letterbox,
            self.model_width,
            self.model_height,
            &mut self.image_input,
        );
        let image = TensorRef::from_array_view((
            [1, 3, self.model_height, self.model_width],
            self.image_input.as_slice(),
        ))
        .map_err(|error| format!("Could not prepare Cutie frame: {error}"))?;
        let key_outputs = self
            .encode_key
            .run(ort::inputs!["image" => image])
            .map_err(|error| format!("Cutie key encoder failed: {error}"))?;
        let features = KeyFeatures {
            f8: output(&key_outputs, "f8")?,
            f4: output(&key_outputs, "f4")?,
            pix_feat: output(&key_outputs, "pix_feat")?,
            key: output(&key_outputs, "key")?,
            shrinkage: output(&key_outputs, "shrinkage")?,
            selection: output(&key_outputs, "selection")?,
        };
        drop(key_outputs);

        let foreground = if let Some(seed) = seed {
            prepare_mask(
                seed,
                letterbox,
                self.model_width,
                self.model_height,
                &mut self.mask_input,
            );
            self.mask_input.clone()
        } else {
            if self.permanent_memory.is_none() {
                return Err("Cutie requires a first-frame seed mask".into());
            }
            self.propagate(&features, control)?
        };

        let should_memorize = seed.is_some()
            || self
                .last_memory_frame
                .is_none_or(|last| self.frame_index.saturating_sub(last) >= MEMORY_EVERY);
        if should_memorize {
            self.memorize(&features, &foreground, letterbox, seed.is_some(), control)?;
            self.last_memory_frame = Some(self.frame_index);
        }
        self.last_mask.clone_from(&foreground);
        self.frame_index += 1;
        restore_mask(&foreground, letterbox, self.model_width, self.model_height)
    }

    fn propagate(
        &mut self,
        features: &KeyFeatures,
        control: &JobControl,
    ) -> Result<Vec<f32>, String> {
        if control.cancelled.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let (memory_key, memory_shrinkage, memory_value, memory_valid) = self.memory_blobs();
        let query_key = TensorRef::from_array_view((
            [1, 64, self.grid_height, self.grid_width],
            features.key.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let query_selection = TensorRef::from_array_view((
            [1, 64, self.grid_height, self.grid_width],
            features.selection.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let memory_key = TensorRef::from_array_view((
            [1, 64, MEMORY_SLOTS, self.grid_height, self.grid_width],
            memory_key.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let memory_shrinkage = TensorRef::from_array_view((
            [1, 1, MEMORY_SLOTS, self.grid_height, self.grid_width],
            memory_shrinkage.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let memory_value = TensorRef::from_array_view((
            [1, 1, 256, MEMORY_SLOTS, self.grid_height, self.grid_width],
            memory_value.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let memory_valid = TensorRef::from_array_view((
            [1, 1, MEMORY_SLOTS, self.grid_height, self.grid_width],
            memory_valid.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let object_memory =
            TensorRef::from_array_view(([1, 1, 1, 16, 257], self.object_memory.as_slice()))
                .map_err(|e| e.to_string())?;
        let pix_feat = TensorRef::from_array_view((
            [1, 256, self.grid_height, self.grid_width],
            features.pix_feat.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let sensory = TensorRef::from_array_view((
            [1, 1, 256, self.grid_height, self.grid_width],
            self.sensory.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let last_mask = TensorRef::from_array_view((
            [1, 1, self.model_height, self.model_width],
            self.last_mask.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let readout = self
            .memory_readout
            .run(ort::inputs![
                "query_key" => query_key,
                "query_selection" => query_selection,
                "memory_key" => memory_key,
                "memory_shrinkage" => memory_shrinkage,
                "memory_value" => memory_value,
                "memory_valid" => memory_valid,
                "object_memory" => object_memory,
                "pix_feat" => pix_feat,
                "sensory" => sensory,
                "last_mask" => last_mask
            ])
            .map_err(|error| format!("Cutie memory readout failed: {error}"))?;
        let memory_readout = output(&readout, "memory_readout")?;
        drop(readout);
        let f8 = TensorRef::from_array_view((
            [1, 512, self.model_height / 8, self.model_width / 8],
            features.f8.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let f4 = TensorRef::from_array_view((
            [1, 256, self.model_height / 4, self.model_width / 4],
            features.f4.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let memory_readout = TensorRef::from_array_view((
            [1, 1, 256, self.grid_height, self.grid_width],
            memory_readout.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let sensory = TensorRef::from_array_view((
            [1, 1, 256, self.grid_height, self.grid_width],
            self.sensory.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let decoded = self
            .decode
            .run(ort::inputs![
                "f8" => f8, "f4" => f4, "memory_readout" => memory_readout, "sensory" => sensory
            ])
            .map_err(|error| format!("Cutie decoder failed: {error}"))?;
        self.sensory = output(&decoded, "new_sensory")?;
        let probability = output(&decoded, "prob")?;
        drop(decoded);
        let plane = self.model_width * self.model_height;
        if probability.len() < plane * 2 {
            return Err("Cutie returned an undersized probability mask".into());
        }
        Ok(probability[plane..plane * 2].to_vec())
    }

    fn memorize(
        &mut self,
        features: &KeyFeatures,
        foreground: &[f32],
        letterbox: Letterbox,
        permanent: bool,
        control: &JobControl,
    ) -> Result<(), String> {
        if control.cancelled.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let image = TensorRef::from_array_view((
            [1, 3, self.model_height, self.model_width],
            self.image_input.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let pix_feat = TensorRef::from_array_view((
            [1, 256, self.grid_height, self.grid_width],
            features.pix_feat.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let sensory = TensorRef::from_array_view((
            [1, 1, 256, self.grid_height, self.grid_width],
            self.sensory.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let mask =
            TensorRef::from_array_view(([1, 1, self.model_height, self.model_width], foreground))
                .map_err(|e| e.to_string())?;
        let values = self
            .encode_value
            .run(ort::inputs![
                "image" => image, "pix_feat" => pix_feat, "sensory" => sensory, "mask" => mask
            ])
            .map_err(|error| format!("Cutie value encoder failed: {error}"))?;
        let frame = MemoryFrame {
            key: features.key.clone(),
            shrinkage: features.shrinkage.clone(),
            value: output(&values, "mask_value")?,
            valid: valid_mask(letterbox, self.grid_width, self.grid_height),
        };
        self.sensory = output(&values, "new_sensory")?;
        let object = output(&values, "object_memory")?;
        drop(values);
        if self.object_memory.is_empty() {
            self.object_memory = object;
        } else {
            for (target, value) in self.object_memory.iter_mut().zip(object) {
                *target += value;
            }
        }
        if permanent || self.permanent_memory.is_none() {
            self.permanent_memory = Some(frame);
        } else {
            self.working_memory.push_back(frame);
            while self.working_memory.len() > MEMORY_SLOTS - 1 {
                self.working_memory.pop_front();
            }
        }
        Ok(())
    }

    fn memory_blobs(&self) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
        let mut keys = vec![0.0; 64 * MEMORY_SLOTS * self.grid_plane];
        let mut shrinkage = vec![0.0; MEMORY_SLOTS * self.grid_plane];
        let mut values = vec![0.0; 256 * MEMORY_SLOTS * self.grid_plane];
        let mut valid = vec![0.0; MEMORY_SLOTS * self.grid_plane];
        let frames = self
            .permanent_memory
            .iter()
            .chain(self.working_memory.iter());
        for (slot, frame) in frames.take(MEMORY_SLOTS).enumerate() {
            copy_channel_slots(&frame.key, &mut keys, slot, 64, self.grid_plane);
            copy_channel_slots(&frame.shrinkage, &mut shrinkage, slot, 1, self.grid_plane);
            copy_value_slots(&frame.value, &mut values, slot, self.grid_plane);
            valid[slot * self.grid_plane..(slot + 1) * self.grid_plane]
                .copy_from_slice(&frame.valid);
        }
        (keys, shrinkage, values, valid)
    }
}

fn copy_channel_slots(
    source: &[f32],
    target: &mut [f32],
    slot: usize,
    channels: usize,
    grid_plane: usize,
) {
    for channel in 0..channels {
        let src = channel * grid_plane;
        let dst = (channel * MEMORY_SLOTS + slot) * grid_plane;
        target[dst..dst + grid_plane].copy_from_slice(&source[src..src + grid_plane]);
    }
}

fn copy_value_slots(source: &[f32], target: &mut [f32], slot: usize, grid_plane: usize) {
    for channel in 0..256 {
        let src = channel * grid_plane;
        let dst = (channel * MEMORY_SLOTS + slot) * grid_plane;
        target[dst..dst + grid_plane].copy_from_slice(&source[src..src + grid_plane]);
    }
}

fn letterbox(width: u32, height: u32, model_width: usize, model_height: usize) -> Letterbox {
    let scale =
        (model_width as f64 / width.max(1) as f64).min(model_height as f64 / height.max(1) as f64);
    let resized_width = (width as f64 * scale)
        .round()
        .clamp(1.0, model_width as f64) as u32;
    let resized_height = (height as f64 * scale)
        .round()
        .clamp(1.0, model_height as f64) as u32;
    Letterbox {
        source_width: width,
        source_height: height,
        x: (model_width as u32 - resized_width) / 2,
        y: (model_height as u32 - resized_height) / 2,
        width: resized_width,
        height: resized_height,
    }
}

fn prepare_image(
    frame: &RgbImage,
    transform: Letterbox,
    model_width: usize,
    model_height: usize,
    output: &mut Vec<f32>,
) {
    let resized = image::imageops::resize(
        frame,
        transform.width,
        transform.height,
        FilterType::Triangle,
    );
    let plane = model_width * model_height;
    output.clear();
    output.resize(plane * 3, 0.0);
    for (x, y, pixel) in resized.enumerate_pixels() {
        let index =
            (y as usize + transform.y as usize) * model_width + x as usize + transform.x as usize;
        for channel in 0..3 {
            output[channel * plane + index] = pixel[channel] as f32 / 255.0;
        }
    }
}

fn prepare_mask(
    mask: &GrayImage,
    transform: Letterbox,
    model_width: usize,
    model_height: usize,
    output: &mut Vec<f32>,
) {
    let resized =
        image::imageops::resize(mask, transform.width, transform.height, FilterType::Nearest);
    output.clear();
    output.resize(model_width * model_height, 0.0);
    for (x, y, pixel) in resized.enumerate_pixels() {
        let index =
            (y as usize + transform.y as usize) * model_width + x as usize + transform.x as usize;
        output[index] = if pixel[0] >= 128 { 1.0 } else { 0.0 };
    }
}

fn valid_mask(transform: Letterbox, grid_width: usize, grid_height: usize) -> Vec<f32> {
    let mut valid = vec![0.0; grid_width * grid_height];
    for y in 0..grid_height {
        for x in 0..grid_width {
            let cx = x as u32 * 16 + 8;
            let cy = y as u32 * 16 + 8;
            if cx >= transform.x
                && cx < transform.x + transform.width
                && cy >= transform.y
                && cy < transform.y + transform.height
            {
                valid[y * grid_width + x] = 1.0;
            }
        }
    }
    valid
}

fn restore_mask(
    foreground: &[f32],
    transform: Letterbox,
    model_width: usize,
    model_height: usize,
) -> Result<GrayImage, String> {
    if foreground.len() < model_width * model_height {
        return Err("Cutie returned an undersized foreground mask".into());
    }
    let model = ImageBuffer::<Luma<u8>, _>::from_fn(transform.width, transform.height, |x, y| {
        let index =
            (y as usize + transform.y as usize) * model_width + x as usize + transform.x as usize;
        Luma([(foreground[index].clamp(0.0, 1.0) * 255.0).round() as u8])
    });
    Ok(image::imageops::resize(
        &model,
        transform.source_width,
        transform.source_height,
        FilterType::CatmullRom,
    ))
}

#[derive(Default)]
pub struct CutieSessionCache {
    active: Mutex<Option<(CutieModelPaths, CutieTracker)>>,
}

impl CutieSessionCache {
    pub fn with_tracker<R>(
        &self,
        paths: CutieModelPaths,
        operation: impl FnOnce(&mut CutieTracker, bool) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut active = self.active.lock();
        let reused = active
            .as_ref()
            .is_some_and(|(cached, _)| cached.encode_key == paths.encode_key && cached.all_exist());
        if !reused {
            *active = None;
            *active = Some((paths.clone(), CutieTracker::load(&paths, true)?));
        }
        let (_, tracker) = active.as_mut().ok_or("Cutie cache was not initialized")?;
        tracker.reset();
        let result = operation(tracker, reused);
        tracker.reset();
        result
    }
    pub fn invalidate(&self) {
        *self.active.lock() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed Cutie models and real media; set ROTO_NOW_MEMORY_TEST_ROOT"]
    fn installed_cache_releases_history_and_handles_tier_load_failure() {
        let root =
            PathBuf::from(std::env::var_os("ROTO_NOW_MEMORY_TEST_ROOT").expect("fixture root"));
        let paths = |tier: &str, width, height| {
            let dir = root.join(".models").join(tier);
            let size = format!("{width}x{height}");
            CutieModelPaths {
                encode_key: dir.join(format!("cutie-encode-key-{size}.onnx")),
                encode_value: dir.join(format!("cutie-encode-value-{size}.onnx")),
                memory_readout: dir.join(format!(
                    "cutie-memory-readout-floatmask-valid-{size}-m6-topk30-opencv.onnx"
                )),
                decode: dir.join(format!("cutie-decode-{size}.onnx")),
                width,
                height,
            }
        };
        let balanced = paths("cutie-medium", 640, 368);
        let high = paths("cutie-high", 960, 544);
        let frame = image::open(root.join(".runtime-test/cutie-first-frame.png"))
            .unwrap()
            .to_rgb8();
        let seed = image::open(root.join(".runtime-test/cutie-seed.png"))
            .unwrap()
            .to_rgba8();
        let seed = image::GrayImage::from_fn(seed.width(), seed.height(), |x, y| {
            image::Luma([seed.get_pixel(x, y)[3]])
        });
        let control = crate::jobs::JobState::default().begin().unwrap();
        let cache = CutieSessionCache::default();
        for (index, model_paths) in [
            balanced.clone(),
            balanced.clone(),
            balanced.clone(),
            high.clone(),
            high,
            balanced,
        ]
        .into_iter()
        .enumerate()
        {
            let result: Result<(), String> = cache.with_tracker(model_paths, |tracker, reused| {
                assert_eq!(reused, matches!(index, 1 | 2 | 4));
                tracker.step(&frame, Some(&seed), &control)?;
                assert!(tracker.permanent_memory.is_some());
                println!(
                    "cache iteration={index} reused={reused} provider={}",
                    tracker.provider()
                );
                if index == 1 {
                    return Err("simulated processing failure".into());
                }
                if index == 2 {
                    return Err("cancelled".into());
                }
                Ok(())
            });
            if matches!(index, 1 | 2) {
                assert!(result.is_err());
            } else {
                result.unwrap();
            }
            let active = cache.active.lock();
            let tracker = &active.as_ref().unwrap().1;
            assert!(tracker.permanent_memory.is_none());
            assert!(tracker.working_memory.is_empty());
            assert!(tracker.last_mask.is_empty());
            assert!(tracker.object_memory.is_empty());
            assert_eq!(tracker.frame_index, 0);
        }
        let missing = paths("missing-tier", 960, 544);
        assert!(cache.with_tracker(missing, |_, _| Ok(())).is_err());
        assert!(cache.active.lock().is_none());
    }

    #[test]
    fn letterbox_preserves_widescreen_geometry() {
        let value = letterbox(1920, 1080, 640, 368);
        assert_eq!((value.width, value.height), (640, 360));
        assert_eq!((value.x, value.y), (0, 4));

        let high = letterbox(1920, 1080, 960, 544);
        assert_eq!((high.width, high.height), (960, 540));
        assert_eq!((high.x, high.y), (0, 2));
    }
    #[test]
    fn memory_blobs_place_channels_in_slot_major_time_axis() {
        let grid_plane = 40 * 23;
        let mut source = vec![0.0; 2 * grid_plane];
        source[grid_plane] = 7.0;
        let mut target = vec![0.0; 2 * MEMORY_SLOTS * grid_plane];
        copy_channel_slots(&source, &mut target, 3, 2, grid_plane);
        assert_eq!(target[(MEMORY_SLOTS + 3) * grid_plane], 7.0);
    }
}
