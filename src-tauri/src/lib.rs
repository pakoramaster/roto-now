pub mod corrections;
pub mod inference;
pub mod jobs;
pub mod models;
pub mod routing;
pub mod temporal;
pub mod video;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use inference::{save_cutout, ModelSessionCache};
use jobs::{emit, emit_progress, JobEvent, JobState, ProcessResult};
use models::ModelId;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, State};

const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_VIDEO_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const STALE_OUTPUT_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineStatus {
    application: &'static str,
    version: &'static str,
    inference_engine: &'static str,
    ffmpeg: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaInfo {
    path: String,
    name: String,
    size: u64,
    kind: &'static str,
    preview_data_url: Option<String>,
}

#[derive(Default)]
struct OutputState {
    outputs: Mutex<HashMap<PathBuf, bool>>,
}

impl Drop for OutputState {
    fn drop(&mut self) {
        for output in self.outputs.get_mut().keys() {
            if is_managed_output_path(output) {
                let _ = fs::remove_file(output);
            }
        }
    }
}

fn extension_kind(path: &Path) -> Option<(&'static str, &'static str)> {
    match path
        .extension()?
        .to_string_lossy()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some(("image", "image/png")),
        "jpg" | "jpeg" => Some(("image", "image/jpeg")),
        "webp" => Some(("image", "image/webp")),
        "mp4" => Some(("video", "video/mp4")),
        "mov" => Some(("video", "video/quicktime")),
        "webm" => Some(("video", "video/webm")),
        _ => None,
    }
}

fn image_data_url(path: &Path, mime: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("Could not read image: {error}"))?;
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn managed_temp_root() -> PathBuf {
    std::env::temp_dir().join("roto-now")
}

fn new_managed_output(extension: &str) -> Result<PathBuf, String> {
    if !matches!(extension, "png" | "mp4") {
        return Err("Unsupported managed output type".into());
    }
    let root = managed_temp_root();
    fs::create_dir_all(&root)
        .map_err(|error| format!("Could not create temporary output folder: {error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_nanos();
    Ok(root.join(format!(
        "result-{}-{timestamp}.{extension}",
        std::process::id()
    )))
}

fn is_managed_output_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    path.parent()
        .is_some_and(|parent| parent == managed_temp_root())
        && name.starts_with("result-")
        && matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("png" | "mp4")
        )
}

fn is_managed_output_file(path: &Path) -> bool {
    is_managed_output_path(path)
        && fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
}

fn cleanup_stale_outputs() {
    let root = managed_temp_root();
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_managed_output_path(&path) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_OUTPUT_AGE);
        if stale {
            let _ = fs::remove_file(path);
        }
    }
}

fn verify_input(path: &str, kind: &str) -> Result<PathBuf, String> {
    let input = PathBuf::from(path);
    if !input.is_file() {
        return Err("The selected input file no longer exists".into());
    }
    if extension_kind(&input).map(|value| value.0) != Some(kind) {
        return Err(format!("A supported {kind} input is required"));
    }
    Ok(input)
}

#[tauri::command]
fn inspect_media(app: AppHandle, path: String) -> Result<MediaInfo, String> {
    let source = PathBuf::from(&path);
    if !app.asset_protocol_scope().is_allowed(&source) {
        return Err("Choose the input through Roto Now's file picker".into());
    }
    let metadata =
        fs::metadata(&source).map_err(|error| format!("Could not inspect file: {error}"))?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".into());
    }
    let (kind, mime) =
        extension_kind(&source).ok_or("Choose a PNG, JPG, WEBP, MP4, MOV, or WEBM file")?;
    let maximum = if kind == "image" {
        MAX_IMAGE_BYTES
    } else {
        MAX_VIDEO_BYTES
    };
    if metadata.len() > maximum {
        let limit = if maximum >= 1024 * 1024 * 1024 {
            format!("{} GB", maximum / 1024 / 1024 / 1024)
        } else {
            format!("{} MB", maximum / 1024 / 1024)
        };
        return Err(format!(
            "The selected {kind} is too large (maximum {limit})"
        ));
    }
    let preview_data_url = (kind == "image")
        .then(|| image_data_url(&source, mime))
        .transpose()?;
    Ok(MediaInfo {
        path,
        name: source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        size: metadata.len(),
        kind,
        preview_data_url,
    })
}

#[tauri::command]
fn start_image_job(
    app: AppHandle,
    jobs: State<'_, JobState>,
    outputs: State<'_, OutputState>,
    input_path: String,
    model: String,
    quality: String,
    edge_detail: u8,
) -> Result<String, String> {
    let input = verify_input(&input_path, "image")?;
    let quality_mode = routing::QualityMode::parse(&quality)?;
    let model_id = routing::select_model(&model, quality_mode)?;
    let model_path = models::model_path(&app, model_id)?;
    if !model_path.is_file() {
        return Err(format!(
            "{} must be downloaded before processing",
            models::spec(model_id).name
        ));
    }
    let output = new_managed_output("png")?;
    let control = jobs.begin()?;
    let job_id = control.id.clone();
    let app_for_task = app.clone();
    outputs.outputs.lock().insert(output.clone(), true);
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let outcome = (|| {
            let source =
                image::open(&input).map_err(|error| format!("Could not open image: {error}"))?;
            app_for_task.state::<ModelSessionCache>().with_model(
                model_path,
                model_id,
                model_id != ModelId::General,
                || {
                    emit_progress(
                        &app_for_task,
                        &control,
                        "loadingModel",
                        None,
                        None,
                        None,
                        "Loading segmentation model",
                    )
                },
                |masker, reused| {
                    emit_progress(
                        &app_for_task,
                        &control,
                        "inference",
                        None,
                        None,
                        None,
                        if reused {
                            "Using loaded model to remove background"
                        } else {
                            "Removing background"
                        },
                    );
                    let cutout = masker.apply(&source, edge_detail, &quality, &control)?;
                    save_cutout(&cutout, &output)?;
                    Ok((
                        masker.provider().to_string(),
                        source.width() as u64 * source.height() as u64,
                    ))
                },
            )
        })();
        match outcome {
            Ok((provider, _)) => emit(
                &app_for_task,
                JobEvent::Completed {
                    job_id: control.id.clone(),
                    result: ProcessResult {
                        output_path: output.to_string_lossy().into_owned(),
                        model: models::spec(model_id).name.into(),
                        provider,
                        duration_ms: started.elapsed().as_millis() as u64,
                        frame_count: None,
                        width: None,
                        height: None,
                        frame_rate: None,
                        media_duration_seconds: None,
                        has_audio: None,
                        preview: false,
                    },
                },
            ),
            Err(error)
                if error == "cancelled"
                    || control.cancelled.load(std::sync::atomic::Ordering::SeqCst) =>
            {
                let _ = fs::remove_file(&output);
                app_for_task
                    .state::<OutputState>()
                    .outputs
                    .lock()
                    .remove(&output);
                emit(
                    &app_for_task,
                    JobEvent::Cancelled {
                        job_id: control.id.clone(),
                    },
                );
            }
            Err(error) => {
                let _ = fs::remove_file(&output);
                app_for_task
                    .state::<OutputState>()
                    .outputs
                    .lock()
                    .remove(&output);
                emit(
                    &app_for_task,
                    JobEvent::Failed {
                        job_id: control.id.clone(),
                        error,
                    },
                );
            }
        }
        app_for_task.state::<JobState>().finish(&control.id);
    });
    Ok(job_id)
}

#[tauri::command]
fn start_video_job(
    app: AppHandle,
    jobs: State<'_, JobState>,
    outputs: State<'_, OutputState>,
    input_path: String,
    model: String,
    quality: String,
    edge_detail: u8,
    screen_color: String,
    preview: bool,
    start_seconds: Option<f64>,
) -> Result<String, String> {
    let input = verify_input(&input_path, "video")?;
    if !matches!(screen_color.as_str(), "green" | "blue") {
        return Err("Screen colour must be green or blue".into());
    }
    let quality_mode = routing::QualityMode::parse(&quality)?;
    let selected_quality = if preview {
        routing::QualityMode::Fast
    } else {
        quality_mode
    };
    let model_id = routing::select_model(&model, selected_quality)?;
    if !models::model_path(&app, model_id)?.is_file() {
        return Err(format!(
            "{} must be downloaded before processing",
            models::spec(model_id).name
        ));
    }
    let output = new_managed_output(if preview { "png" } else { "mp4" })?;
    let control = jobs.begin()?;
    let job_id = control.id.clone();
    let app_for_task = app.clone();
    outputs.outputs.lock().insert(output.clone(), !preview);
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let outcome = video::process_video(
            &app_for_task,
            &control,
            &input,
            &output,
            model_id,
            edge_detail,
            &quality,
            &screen_color,
            preview,
            start_seconds.unwrap_or(0.0),
        );
        match outcome {
            Ok(value) => emit(
                &app_for_task,
                JobEvent::Completed {
                    job_id: control.id.clone(),
                    result: ProcessResult {
                        output_path: output.to_string_lossy().into_owned(),
                        model: models::spec(model_id).name.into(),
                        provider: value.provider,
                        duration_ms: started.elapsed().as_millis() as u64,
                        frame_count: Some(value.frame_count),
                        width: Some(value.width),
                        height: Some(value.height),
                        frame_rate: Some(value.frame_rate),
                        media_duration_seconds: Some(value.duration),
                        has_audio: Some(value.has_audio),
                        preview,
                    },
                },
            ),
            Err(error)
                if error == "cancelled"
                    || control.cancelled.load(std::sync::atomic::Ordering::SeqCst) =>
            {
                let _ = fs::remove_file(&output);
                app_for_task
                    .state::<OutputState>()
                    .outputs
                    .lock()
                    .remove(&output);
                emit(
                    &app_for_task,
                    JobEvent::Cancelled {
                        job_id: control.id.clone(),
                    },
                );
            }
            Err(error) => {
                let _ = fs::remove_file(&output);
                app_for_task
                    .state::<OutputState>()
                    .outputs
                    .lock()
                    .remove(&output);
                emit(
                    &app_for_task,
                    JobEvent::Failed {
                        job_id: control.id.clone(),
                        error,
                    },
                );
            }
        }
        app_for_task.state::<JobState>().finish(&control.id);
    });
    Ok(job_id)
}

#[tauri::command]
fn save_output(
    app: AppHandle,
    outputs: State<'_, OutputState>,
    source_path: String,
    destination_path: String,
) -> Result<String, String> {
    let source = PathBuf::from(&source_path);
    if outputs.outputs.lock().get(&source) != Some(&true) {
        return Err("Preview outputs cannot be saved; run the full export first".into());
    }
    if !is_managed_output_file(&source) {
        return Err("Only Roto Now temporary results can be saved".into());
    }
    let destination = PathBuf::from(&destination_path);
    if !app.asset_protocol_scope().is_allowed(&destination) {
        return Err("Choose the destination through Roto Now's save dialog".into());
    }
    if source
        .extension()
        .map(|v| v.to_string_lossy().to_ascii_lowercase())
        != destination
            .extension()
            .map(|v| v.to_string_lossy().to_ascii_lowercase())
    {
        return Err("The saved file must keep the processed result extension".into());
    }
    fs::copy(&source, &destination).map_err(|error| format!("Could not save result: {error}"))?;
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
fn apply_image_corrections(
    outputs: State<'_, OutputState>,
    source_path: String,
    strokes: Vec<corrections::CorrectionStroke>,
) -> Result<String, String> {
    let source = PathBuf::from(&source_path);
    if outputs.outputs.lock().get(&source) != Some(&true)
        || !is_managed_output_file(&source)
        || source.extension().and_then(|value| value.to_str()) != Some("png")
    {
        return Err("Only a managed PNG result can be corrected".into());
    }

    let mut image = image::open(&source)
        .map_err(|error| format!("Could not open the cutout for correction: {error}"))?
        .to_rgba8();
    corrections::apply_corrections(&mut image, &strokes)?;
    let output = new_managed_output("png")?;
    image
        .save_with_format(&output, image::ImageFormat::Png)
        .map_err(|error| format!("Could not save the corrected cutout: {error}"))?;
    outputs.outputs.lock().insert(output.clone(), true);
    Ok(output.to_string_lossy().into_owned())
}

#[tauri::command]
fn discard_output(outputs: State<'_, OutputState>, path: String) -> Result<(), String> {
    let output = PathBuf::from(path);
    let mut registered = outputs.outputs.lock();
    if !registered.contains_key(&output) {
        return Ok(());
    }
    if !is_managed_output_path(&output) {
        return Err("Refusing to discard a path outside Roto Now's temporary outputs".into());
    }
    if output.is_file() {
        fs::remove_file(&output)
            .map_err(|error| format!("Could not discard temporary result: {error}"))?;
    }
    registered.remove(&output);
    Ok(())
}

#[tauri::command]
fn engine_status() -> EngineStatus {
    EngineStatus {
        application: "ready",
        version: env!("CARGO_PKG_VERSION"),
        inference_engine: "Native ONNX Runtime",
        ffmpeg: "bundled",
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    cleanup_stale_outputs();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(JobState::default())
        .manage(OutputState::default())
        .manage(ModelSessionCache::default())
        .invoke_handler(tauri::generate_handler![
            engine_status,
            inspect_media,
            models::get_bootstrap_status,
            models::download_model,
            models::remove_model,
            models::cancel_job,
            start_image_job,
            start_video_job,
            apply_image_corrections,
            save_output,
            discard_output
        ])
        .run(tauri::generate_context!())
        .expect("error while running Roto Now");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_extensions_are_allowlisted_case_insensitively() {
        assert_eq!(extension_kind(Path::new("photo.PNG")).unwrap().0, "image");
        assert_eq!(extension_kind(Path::new("clip.MOV")).unwrap().0, "video");
        assert!(extension_kind(Path::new("payload.exe")).is_none());
        assert!(extension_kind(Path::new("no-extension")).is_none());
    }

    #[test]
    fn managed_output_policy_rejects_nested_and_unrelated_paths() {
        let root = managed_temp_root();
        assert!(is_managed_output_path(&root.join("result-10-20.png")));
        assert!(is_managed_output_path(&root.join("result-10-20.mp4")));
        assert!(!is_managed_output_path(&root.join("unrelated.png")));
        assert!(!is_managed_output_path(
            &root.join("nested").join("result-10-20.png")
        ));
        assert!(!is_managed_output_path(
            &root
                .with_file_name("roto-now-elsewhere")
                .join("result-10-20.png")
        ));
    }

    #[test]
    fn output_state_removes_registered_managed_files_on_drop() {
        let root = managed_temp_root();
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!(
            "result-test-{}.png",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"temporary result").unwrap();
        let state = OutputState::default();
        state.outputs.lock().insert(path.clone(), true);
        drop(state);
        assert!(!path.exists());
    }
}
