use crate::jobs::{emit, emit_progress, JobEvent, JobState};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, OnceLock},
};
use tauri::{AppHandle, Manager, State};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ModelId {
    GeneralLite,
    General,
    Anime,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: ModelId,
    pub name: &'static str,
    pub size: u64,
    pub installed: bool,
    pub managed: bool,
    pub state: &'static str,
    pub provider: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapStatus {
    pub ready: bool,
    pub provider: &'static str,
    pub models: Vec<ModelStatus>,
    pub trackers: Vec<crate::cutie_models::CutieStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveMember {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    pub id: ModelId,
    pub name: String,
    pub role: String,
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub destination: String,
    pub archive_members: Vec<ArchiveMember>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutieSpec {
    pub id: String,
    pub name: String,
    pub role: String,
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub destination: String,
    pub width: usize,
    pub height: usize,
    pub archive_members: Vec<ArchiveMember>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelManifest {
    schema_version: u32,
    models: Vec<ModelSpec>,
    cutie: CutieSpec,
}

fn manifest() -> &'static ModelManifest {
    static MANIFEST: OnceLock<ModelManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        let parsed: ModelManifest = serde_json::from_str(include_str!("../model-manifest.json"))
            .expect("the embedded model manifest is valid");
        assert_eq!(
            parsed.schema_version, 1,
            "unsupported model manifest schema"
        );
        parsed
    })
}

pub fn specs() -> &'static [ModelSpec] {
    &manifest().models
}
pub fn spec(id: ModelId) -> &'static ModelSpec {
    specs()
        .iter()
        .find(|item| item.id == id)
        .expect("model registry is complete")
}
pub fn cutie_spec() -> &'static CutieSpec {
    &manifest().cutie
}

pub fn model_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate application data: {error}"))?
        .join("models");
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("Could not create model folder: {error}"))?;
    Ok(root)
}

pub fn model_path(app: &AppHandle, id: ModelId) -> Result<PathBuf, String> {
    let managed = managed_model_path(app, id)?;
    if managed.is_file() {
        return Ok(managed);
    }
    #[cfg(debug_assertions)]
    if let Some(local) = development_model_path(id) {
        if local.is_file() {
            return Ok(local);
        }
    }
    Ok(managed)
}

fn managed_model_path(app: &AppHandle, id: ModelId) -> Result<PathBuf, String> {
    Ok(model_root(app)?.join(&spec(id).destination))
}

#[cfg(debug_assertions)]
fn development_model_path(id: ModelId) -> Option<PathBuf> {
    let root = std::env::var_os("ROTO_NOW_MODEL_ROOT").map(PathBuf::from)?;
    Some(development_model_path_from_root(&root, id))
}

#[cfg(any(debug_assertions, test))]
fn development_model_path_from_root(root: &Path, id: ModelId) -> PathBuf {
    let folder = if id == ModelId::Anime {
        "toonout"
    } else {
        "rembg"
    };
    root.join(folder).join(&spec(id).destination)
}

fn status_for(app: &AppHandle, item: &'static ModelSpec) -> ModelStatus {
    let managed_path = managed_model_path(app, item.id).ok();
    let path = model_path(app, item.id).ok();
    let installed = path.as_ref().is_some_and(|path| {
        if !path.is_file() {
            return false;
        }
        if item.role != "required" {
            return true;
        }
        path.metadata()
            .is_ok_and(|metadata| metadata.len() == item.size)
            && sha256_file(path).is_ok_and(|hash| hash == item.sha256)
    });
    let managed = installed
        && path
            .as_ref()
            .zip(managed_path.as_ref())
            .is_some_and(|(resolved, managed)| resolved == managed);
    let partial = managed_path
        .as_ref()
        .is_some_and(|path| path.with_extension("onnx.part").is_file());
    ModelStatus {
        id: item.id,
        name: item.name.as_str(),
        size: item.size,
        installed,
        managed,
        state: if installed && !managed {
            "local"
        } else if installed {
            "ready"
        } else if partial {
            "partial"
        } else {
            "missing"
        },
        provider: if item.id == ModelId::General {
            "CPU / NVENC for video"
        } else {
            "DirectML / CPU"
        },
    }
}

#[tauri::command]
pub fn get_bootstrap_status(app: AppHandle) -> Result<BootstrapStatus, String> {
    let models: Vec<_> = specs().iter().map(|item| status_for(&app, item)).collect();
    Ok(BootstrapStatus {
        ready: models
            .iter()
            .any(|item| item.id == ModelId::GeneralLite && item.installed),
        provider: "DirectML with CPU fallback",
        models,
        trackers: crate::cutie_models::statuses(&app),
    })
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Could not verify model: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not verify model: {error}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

async fn download_once(
    app: &AppHandle,
    control: &crate::jobs::JobControl,
    item: &'static ModelSpec,
    destination: &Path,
) -> Result<(), String> {
    let partial = destination.with_extension("onnx.part");
    let mut existing = tokio::fs::metadata(&partial)
        .await
        .map(|value| value.len())
        .unwrap_or(0);
    if existing > item.size {
        let _ = tokio::fs::remove_file(&partial).await;
        existing = 0;
    } else if existing == item.size {
        emit_progress(
            app,
            control,
            "verifying",
            None,
            None,
            None,
            format!("Verifying {}", item.name),
        );
        if sha256_file(&partial)? == item.sha256 {
            atomic_replace(&partial, destination)?;
            return Ok(());
        }
        let _ = tokio::fs::remove_file(&partial).await;
        existing = 0;
    }
    let client = reqwest::Client::builder()
        .user_agent("RotoNow/0.5")
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(&item.url);
    if existing > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Download failed: {error}"))?;
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        return Err(format!("Model server returned {}", response.status()));
    }
    let start = if resumed { existing } else { 0 };
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&partial)
        .await
        .map_err(|error| format!("Could not open partial model: {error}"))?;
    let total = response
        .content_length()
        .map(|length| start + length)
        .unwrap_or(item.size);
    let mut downloaded = start;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if control.cancelled.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let chunk = chunk.map_err(|error| format!("Download interrupted: {error}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("Could not write model: {error}"))?;
        downloaded += chunk.len() as u64;
        emit_progress(
            app,
            control,
            "downloading",
            Some(downloaded),
            Some(total),
            None,
            format!("Downloading {}", item.name),
        );
    }
    file.flush()
        .await
        .map_err(|error| format!("Could not finish model: {error}"))?;
    drop(file);
    emit_progress(
        app,
        control,
        "verifying",
        None,
        None,
        None,
        format!("Verifying {}", item.name),
    );
    if sha256_file(&partial)? != item.sha256 {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err("Downloaded model failed checksum verification".into());
    }
    atomic_replace(&partial, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(format!(
            "Could not install model: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(source, destination)
        .map_err(|error| format!("Could not install model: {error}"))
}

#[tauri::command]
pub fn download_model(
    app: AppHandle,
    state: State<'_, JobState>,
    cache: State<'_, crate::inference::ModelSessionCache>,
    model_id: ModelId,
) -> Result<String, String> {
    let destination = managed_model_path(&app, model_id)?;
    let control = state.begin()?;
    cache.invalidate(model_id);
    let job_id = control.id.clone();
    let item = spec(model_id);
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut outcome = Err("Download did not start".to_string());
        for attempt in 1..=3 {
            outcome = download_once(&app_for_task, &control, item, &destination).await;
            if outcome.is_ok() || control.cancelled.load(Ordering::SeqCst) {
                break;
            }
            if attempt < 3 {
                emit_progress(
                    &app_for_task,
                    &control,
                    "retrying",
                    None,
                    None,
                    None,
                    format!("Retrying download ({}/{})", attempt + 1, 3),
                );
            }
        }
        if control.cancelled.load(Ordering::SeqCst) {
            emit(
                &app_for_task,
                JobEvent::Cancelled {
                    job_id: control.id.clone(),
                },
            );
        } else if let Err(error) = outcome {
            emit(
                &app_for_task,
                JobEvent::Failed {
                    job_id: control.id.clone(),
                    error,
                },
            );
        } else {
            emit(
                &app_for_task,
                JobEvent::Completed {
                    job_id: control.id.clone(),
                    result: crate::jobs::ProcessResult::model_download(item.name.as_str()),
                },
            );
        }
        app_for_task.state::<JobState>().finish(&control.id);
    });
    Ok(job_id)
}

#[tauri::command]
pub fn remove_model(
    app: AppHandle,
    cache: State<'_, crate::inference::ModelSessionCache>,
    model_id: ModelId,
) -> Result<(), String> {
    if spec(model_id).role == "required" {
        return Err("General is required and cannot be removed".into());
    }
    cache.invalidate(model_id);
    let path = managed_model_path(&app, model_id)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| format!("Could not remove model: {error}"))?;
    }
    let partial = path.with_extension("onnx.part");
    if partial.exists() {
        std::fs::remove_file(partial)
            .map_err(|error| format!("Could not remove partial model: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_job(state: State<'_, JobState>, job_id: String) -> Result<(), String> {
    state.cancel(&job_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_has_required_general_and_optional_models() {
        assert_eq!(specs().len(), 3);
        assert_eq!(spec(ModelId::GeneralLite).role, "required");
        assert_eq!(
            spec(ModelId::GeneralLite).destination,
            "birefnet-general-lite-fp16.onnx"
        );
        assert_eq!(spec(ModelId::General).role, "optional");
        assert_eq!(spec(ModelId::Anime).role, "optional");
        for model in specs() {
            assert_eq!(model.sha256.len(), 64);
            assert!(model.size > 10_000_000);
        }
    }
    #[test]
    fn development_models_follow_reference_layout() {
        let root = Path::new("test-models");
        assert_eq!(
            development_model_path_from_root(root, ModelId::GeneralLite),
            root.join("rembg").join("birefnet-general-lite-fp16.onnx")
        );
        assert_eq!(
            development_model_path_from_root(root, ModelId::General),
            root.join("rembg").join("birefnet-general.onnx")
        );
        assert_eq!(
            development_model_path_from_root(root, ModelId::Anime),
            root.join("toonout").join("birefnet-toonout-fp16.onnx")
        );
    }
}
