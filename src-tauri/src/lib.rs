pub mod inference;
pub mod jobs;
pub mod models;
pub mod video;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use inference::{save_cutout, Masker};
use jobs::{emit, emit_progress, JobEvent, JobState, ProcessResult};
use models::ModelId;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, State};

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

fn selected_model(model: &str, quality: &str, preview: bool) -> ModelId {
    if model.eq_ignore_ascii_case("anime") {
        ModelId::Anime
    } else if !preview && quality.eq_ignore_ascii_case("maximum") {
        ModelId::General
    } else {
        ModelId::GeneralLite
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
fn inspect_media(path: String) -> Result<MediaInfo, String> {
    let source = PathBuf::from(&path);
    let metadata =
        fs::metadata(&source).map_err(|error| format!("Could not inspect file: {error}"))?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".into());
    }
    let (kind, mime) =
        extension_kind(&source).ok_or("Choose a PNG, JPG, WEBP, MP4, MOV, or WEBM file")?;
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
    let model_id = selected_model(&model, &quality, false);
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
        emit_progress(
            &app_for_task,
            &control,
            "loadingModel",
            None,
            None,
            None,
            "Loading segmentation model",
        );
        let outcome = (|| {
            let source =
                image::open(&input).map_err(|error| format!("Could not open image: {error}"))?;
            let mut masker = Masker::load(&app_for_task, model_id, model_id != ModelId::General)?;
            emit_progress(
                &app_for_task,
                &control,
                "inference",
                None,
                None,
                None,
                "Removing background",
            );
            let cutout = masker.apply(&source, edge_detail, &control)?;
            save_cutout(&cutout, &output)?;
            Ok::<_, String>((
                masker.provider().to_string(),
                source.width() as u64 * source.height() as u64,
            ))
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
    let model_id = selected_model(&model, &quality, preview);
    if !models::model_path(&app, model_id)?.is_file() {
        return Err(format!(
            "{} must be downloaded before processing",
            models::spec(model_id).name
        ));
    }
    let output = new_managed_output("mp4")?;
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
    outputs: State<'_, OutputState>,
    source_path: String,
    destination_path: String,
) -> Result<String, String> {
    let source = PathBuf::from(&source_path);
    if outputs.outputs.lock().get(&source) != Some(&true) {
        return Err("Preview outputs cannot be saved; run the full export first".into());
    }
    if !source.is_file() || !source.starts_with(managed_temp_root()) {
        return Err("Only Roto Now temporary results can be saved".into());
    }
    let destination = PathBuf::from(&destination_path);
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
fn discard_output(outputs: State<'_, OutputState>, path: String) -> Result<(), String> {
    let output = PathBuf::from(path);
    if outputs.outputs.lock().remove(&output).is_none() {
        return Ok(());
    }
    if output.is_file() {
        fs::remove_file(output)
            .map_err(|error| format!("Could not discard temporary result: {error}"))?;
    }
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
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(JobState::default())
        .manage(OutputState::default())
        .invoke_handler(tauri::generate_handler![
            engine_status,
            inspect_media,
            models::get_bootstrap_status,
            models::download_model,
            models::remove_model,
            models::cancel_job,
            start_image_job,
            start_video_job,
            save_output,
            discard_output
        ])
        .run(tauri::generate_context!())
        .expect("error while running Roto Now");
}
