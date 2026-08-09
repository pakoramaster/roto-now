use crate::jobs::{emit, emit_progress, JobEvent, JobState};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
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
}

pub struct ModelSpec {
    pub id: ModelId,
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

pub const MODEL_SPECS: [ModelSpec; 3] = [
    ModelSpec {
        id: ModelId::GeneralLite,
        name: "General Lite",
        file: "birefnet-general-lite.onnx",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx",
        sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
        size: 224_005_088,
    },
    ModelSpec {
        id: ModelId::General,
        name: "General Maximum",
        file: "birefnet-general.onnx",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-epoch_244.onnx",
        sha256: "58f621f00f5d756097615970a88a791584600dcf7c45b18a0a6267535a1ebd3c",
        size: 972_666_916,
    },
    ModelSpec {
        id: ModelId::Anime,
        name: "Anime ToonOut",
        file: "birefnet-toonout-fp16.onnx",
        url: "https://huggingface.co/sprited/birefnet-toonout-onnx/resolve/main/birefnet-toonout-fp16.onnx?download=true",
        sha256: "213a8a98ee426ef8f02d247eb5a5a9889359e37c2e1e7e31e282d61034d08a83",
        size: 492_381_880,
    },
];

pub fn spec(id: ModelId) -> &'static ModelSpec {
    MODEL_SPECS
        .iter()
        .find(|item| item.id == id)
        .expect("model registry is complete")
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
    Ok(model_root(app)?.join(spec(id).file))
}

#[cfg(debug_assertions)]
fn development_model_path(id: ModelId) -> Option<PathBuf> {
    let root = std::env::var_os("ROTO_NOW_MODEL_ROOT").map(PathBuf::from)?;
    Some(development_model_path_from_root(&root, id))
}

fn development_model_path_from_root(root: &Path, id: ModelId) -> PathBuf {
    let folder = if id == ModelId::Anime {
        "toonout"
    } else {
        "rembg"
    };
    root.join(folder).join(spec(id).file)
}

fn seed_bundled_general_lite(app: &AppHandle) -> Result<(), String> {
    let root = model_root(app)?;
    let marker = root.join(".bundled-general-lite-seeded");
    if marker.is_file() {
        return Ok(());
    }

    let destination = root.join(spec(ModelId::GeneralLite).file);
    if destination.is_file() && sha256_file(&destination)? == spec(ModelId::GeneralLite).sha256 {
        std::fs::write(marker, b"1")
            .map_err(|error| format!("Could not finish bundled model setup: {error}"))?;
        return Ok(());
    }

    let bundled = app
        .path()
        .resource_dir()
        .map_err(|error| format!("Could not locate bundled resources: {error}"))?
        .join("models")
        .join(spec(ModelId::GeneralLite).file);
    if !bundled.is_file() {
        return Ok(());
    }
    if sha256_file(&bundled)? != spec(ModelId::GeneralLite).sha256 {
        return Err("The bundled General Lite model failed checksum verification".into());
    }

    let partial = destination.with_extension("onnx.part");
    std::fs::copy(&bundled, &partial)
        .map_err(|error| format!("Could not install bundled General Lite: {error}"))?;
    if sha256_file(&partial)? != spec(ModelId::GeneralLite).sha256 {
        let _ = std::fs::remove_file(&partial);
        return Err("The installed General Lite copy failed checksum verification".into());
    }
    atomic_replace(&partial, &destination)?;
    std::fs::write(marker, b"1")
        .map_err(|error| format!("Could not finish bundled model setup: {error}"))?;
    Ok(())
}

fn status_for(app: &AppHandle, item: &'static ModelSpec) -> ModelStatus {
    let managed_path = managed_model_path(app, item.id).ok();
    let path = model_path(app, item.id).ok();
    let installed = path.as_ref().map(|path| path.is_file()).unwrap_or(false);
    let managed = installed
        && path
            .as_ref()
            .zip(managed_path.as_ref())
            .is_some_and(|(resolved, managed)| resolved == managed);
    let partial = managed_path
        .as_ref()
        .map(|path| path.with_extension("onnx.part").is_file())
        .unwrap_or(false);
    ModelStatus {
        id: item.id,
        name: item.name,
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
            "CPU fallback"
        } else {
            "DirectML / CPU"
        },
    }
}

#[tauri::command]
pub fn get_bootstrap_status(app: AppHandle) -> Result<BootstrapStatus, String> {
    seed_bundled_general_lite(&app)?;
    let models: Vec<_> = MODEL_SPECS
        .iter()
        .map(|item| status_for(&app, item))
        .collect();
    Ok(BootstrapStatus {
        ready: models
            .iter()
            .any(|item| item.id == ModelId::GeneralLite && item.installed),
        provider: "DirectML with CPU fallback",
        models,
    })
}

fn sha256_file(path: &Path) -> Result<String, String> {
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
        .user_agent("RotoNow/0.1")
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(item.url);
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
    let actual = sha256_file(&partial)?;
    if actual != item.sha256 {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err("Downloaded model failed checksum verification".into());
    }
    atomic_replace(&partial, destination)?;
    Ok(())
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
            let _ = tokio::fs::remove_file(destination.with_extension("onnx.part")).await;
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
                    result: crate::jobs::ProcessResult {
                        output_path: String::new(),
                        model: item.name.into(),
                        provider: "installed".into(),
                        duration_ms: 0,
                        frame_count: None,
                        width: None,
                        height: None,
                        frame_rate: None,
                        media_duration_seconds: None,
                        has_audio: None,
                        preview: false,
                    },
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
    fn registry_has_pinned_sha256_values() {
        assert_eq!(MODEL_SPECS.len(), 3);
        for model in MODEL_SPECS.iter() {
            assert_eq!(model.sha256.len(), 64);
            assert!(model.sha256.bytes().all(|value| value.is_ascii_hexdigit()));
            assert!(model.size > 200_000_000);
        }
    }

    #[test]
    fn development_models_follow_the_reference_folder_layout() {
        let root = Path::new("test-models");
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
