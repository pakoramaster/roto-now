use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerResult {
    ok: bool,
    output_path: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    duration_ms: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageProcessResult {
    output_path: String,
    model: String,
    provider: String,
    duration_ms: u64,
    preview_data_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoWorkerResult {
    ok: bool,
    output_path: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    duration_ms: Option<u64>,
    frame_count: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoProcessResult {
    output_path: String,
    model: String,
    provider: String,
    duration_ms: u64,
    frame_count: u64,
}

fn extension_kind(path: &Path) -> Option<(&'static str, &'static str)> {
    match path.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
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
    fs::create_dir_all(&root).map_err(|error| format!("Could not create temporary output folder: {error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_nanos();
    Ok(root.join(format!("preview-{}-{timestamp}.{extension}", std::process::id())))
}

fn verified_managed_output(path: &Path) -> Result<PathBuf, String> {
    let root = managed_temp_root();
    fs::create_dir_all(&root).map_err(|error| format!("Could not access temporary output folder: {error}"))?;
    let canonical_root = root.canonicalize().map_err(|error| format!("Could not verify temporary folder: {error}"))?;
    let canonical_path = path.canonicalize().map_err(|error| format!("Could not find temporary result: {error}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("Only Roto Now temporary results can be saved or discarded".into());
    }
    Ok(canonical_path)
}

#[tauri::command]
fn inspect_media(path: String) -> Result<MediaInfo, String> {
    let source = PathBuf::from(&path);
    let metadata = fs::metadata(&source).map_err(|error| format!("Could not inspect file: {error}"))?;
    if !metadata.is_file() {
        return Err("The selected path is not a file".into());
    }
    let (kind, mime) = extension_kind(&source).ok_or("Choose a PNG, JPG, WEBP, MP4, MOV, or WEBM file")?;
    let preview_data_url = if kind == "image" { Some(image_data_url(&source, mime)?) } else { None };
    Ok(MediaInfo {
        path,
        name: source.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        size: metadata.len(),
        kind,
        preview_data_url,
    })
}

#[tauri::command]
async fn process_image(
    input_path: String,
    output_path: Option<String>,
    model: String,
    quality: String,
    edge_detail: u8,
) -> Result<ImageProcessResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let input = PathBuf::from(&input_path);
        let output = match output_path {
            Some(path) => PathBuf::from(path),
            None => new_managed_output("png")?,
        };
        if extension_kind(&input).map(|value| value.0) != Some("image") {
            return Err("Image processing requires a PNG, JPG, or WEBP input".into());
        }
        if output.extension().map(|value| value.to_string_lossy().to_ascii_lowercase()) != Some("png".into()) {
            return Err("Output must be a .png file".into());
        }

        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("Could not locate the project root")?
            .to_path_buf();
        let python = project_root.join(".python-env").join("Scripts").join("python.exe");
        let worker = project_root.join("backend").join("worker.py");
        let models = project_root.join(".models");
        if !python.is_file() {
            return Err("The inference environment is missing. Run scripts/setup-inference.ps1 first.".into());
        }

        let started = Instant::now();
        let process = Command::new(&python)
            .arg(&worker)
            .arg("--input").arg(&input)
            .arg("--output").arg(&output)
            .arg("--model").arg(model.to_ascii_lowercase())
            .arg("--quality").arg(quality.to_ascii_lowercase())
            .arg("--edge-detail").arg(edge_detail.to_string())
            .arg("--models-dir").arg(&models)
            .current_dir(&project_root)
            .output()
            .map_err(|error| format!("Could not start inference worker: {error}"))?;

        let stdout = String::from_utf8_lossy(&process.stdout);
        let worker_result = stdout.lines().rev().find_map(|line| serde_json::from_str::<WorkerResult>(line).ok());
        let stderr = String::from_utf8_lossy(&process.stderr);
        let result = worker_result.ok_or_else(|| format!("Inference worker returned an invalid response. {stderr}"))?;
        if !process.status.success() || !result.ok {
            return Err(result.error.unwrap_or_else(|| format!("Inference failed. {stderr}")));
        }
        if !output.is_file() {
            return Err("Inference completed without creating an output file".into());
        }

        Ok(ImageProcessResult {
            output_path: result.output_path.unwrap_or_else(|| output.to_string_lossy().into_owned()),
            model: result.model.unwrap_or(model),
            provider: result.provider.unwrap_or_else(|| "CPUExecutionProvider".into()),
            duration_ms: result.duration_ms.unwrap_or_else(|| started.elapsed().as_millis() as u64),
            preview_data_url: image_data_url(&output, "image/png")?,
        })
    }).await.map_err(|error| format!("Inference task failed: {error}"))?
}

#[tauri::command]
async fn process_video(
    input_path: String,
    output_path: Option<String>,
    model: String,
    quality: String,
    edge_detail: u8,
    screen_color: String,
) -> Result<VideoProcessResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let input = PathBuf::from(&input_path);
        let output = match output_path {
            Some(path) => PathBuf::from(path),
            None => new_managed_output("mp4")?,
        };
        if extension_kind(&input).map(|value| value.0) != Some("video") {
            return Err("Video processing requires an MP4, MOV, or WEBM input".into());
        }
        if output.extension().map(|value| value.to_string_lossy().to_ascii_lowercase()) != Some("mp4".into()) {
            return Err("Output must be a .mp4 file".into());
        }
        if !matches!(screen_color.as_str(), "green" | "blue") {
            return Err("Screen color must be green or blue".into());
        }

        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("Could not locate the project root")?
            .to_path_buf();
        let python = project_root.join(".python-env").join("Scripts").join("python.exe");
        let worker = project_root.join("backend").join("video_worker.py");
        let models = project_root.join(".models");
        if !python.is_file() {
            return Err("The inference environment is missing. Run scripts/setup-inference.ps1 first.".into());
        }

        let started = Instant::now();
        let process = Command::new(&python)
            .arg(&worker)
            .arg("--input").arg(&input)
            .arg("--output").arg(&output)
            .arg("--model").arg(model.to_ascii_lowercase())
            .arg("--quality").arg(quality.to_ascii_lowercase())
            .arg("--edge-detail").arg(edge_detail.to_string())
            .arg("--screen-color").arg(screen_color)
            .arg("--models-dir").arg(&models)
            .current_dir(&project_root)
            .output()
            .map_err(|error| format!("Could not start video worker: {error}"))?;

        let stdout = String::from_utf8_lossy(&process.stdout);
        let worker_result = stdout.lines().rev().find_map(|line| serde_json::from_str::<VideoWorkerResult>(line).ok());
        let stderr = String::from_utf8_lossy(&process.stderr);
        let result = worker_result.ok_or_else(|| format!("Video worker returned an invalid response. {stderr}"))?;
        if !process.status.success() || !result.ok {
            return Err(result.error.unwrap_or_else(|| format!("Video processing failed. {stderr}")));
        }
        if !output.is_file() {
            return Err("Video processing completed without creating an output file".into());
        }

        Ok(VideoProcessResult {
            output_path: result.output_path.unwrap_or_else(|| output.to_string_lossy().into_owned()),
            model: result.model.unwrap_or(model),
            provider: result.provider.unwrap_or_else(|| "CPUExecutionProvider".into()),
            duration_ms: result.duration_ms.unwrap_or_else(|| started.elapsed().as_millis() as u64),
            frame_count: result.frame_count.unwrap_or(0),
        })
    }).await.map_err(|error| format!("Video task failed: {error}"))?
}

#[tauri::command]
fn save_output(source_path: String, destination_path: String) -> Result<String, String> {
    let source = verified_managed_output(Path::new(&source_path))?;
    let destination = PathBuf::from(&destination_path);
    if source.extension().map(|value| value.to_string_lossy().to_ascii_lowercase())
        != destination.extension().map(|value| value.to_string_lossy().to_ascii_lowercase())
    {
        return Err("The saved file must keep the same extension as the processed result".into());
    }
    fs::copy(&source, &destination).map_err(|error| format!("Could not save result: {error}"))?;
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
fn discard_output(path: String) -> Result<(), String> {
    let output = verified_managed_output(Path::new(&path))?;
    fs::remove_file(output).map_err(|error| format!("Could not discard temporary result: {error}"))
}

#[tauri::command]
fn engine_status() -> EngineStatus {
    EngineStatus {
        application: "ready",
        version: env!("CARGO_PKG_VERSION"),
        inference_engine: "BiRefNet / ToonOut ONNX",
        ffmpeg: "bundled adapter ready",
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            engine_status,
            inspect_media,
            process_image,
            process_video,
            save_output,
            discard_output
        ])
        .run(tauri::generate_context!())
        .expect("error while running Roto Now");
}
