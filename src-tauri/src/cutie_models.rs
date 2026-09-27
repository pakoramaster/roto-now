use crate::{
    cutie::CutieModelPaths,
    jobs::{emit, emit_progress, JobEvent, JobState, ProcessResult},
};
use futures_util::StreamExt;
use serde::Serialize;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
use tauri::{AppHandle, Manager, State};
use tokio::io::AsyncWriteExt;

use crate::models::CutieSpec;

fn files(spec: &CutieSpec) -> Vec<String> {
    spec.archive_members
        .iter()
        .map(|member| member.path.clone())
        .collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutieStatus {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub width: usize,
    pub height: usize,
    pub installed: bool,
    pub managed: bool,
    pub state: &'static str,
    pub provider: &'static str,
}

fn managed_dir(app: &AppHandle, spec: &CutieSpec) -> Result<PathBuf, String> {
    Ok(crate::models::model_root(app)?.join(&spec.destination))
}

fn paths_in(root: &Path, spec: &CutieSpec) -> CutieModelPaths {
    let names = files(spec);
    CutieModelPaths {
        encode_key: root.join(&names[0]),
        encode_value: root.join(&names[1]),
        memory_readout: root.join(&names[2]),
        decode: root.join(&names[3]),
        width: spec.width,
        height: spec.height,
    }
}

pub fn model_paths(app: &AppHandle) -> Result<CutieModelPaths, String> {
    let spec = crate::models::cutie_spec();
    let managed = paths_in(&managed_dir(app, spec)?, spec);
    if managed.all_exist() {
        return Ok(managed);
    }
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("ROTO_NOW_MODEL_ROOT") {
        let local = paths_in(&PathBuf::from(root).join(&spec.destination), spec);
        if local.all_exist() {
            return Ok(local);
        }
    }
    Ok(managed)
}

fn status(app: &AppHandle, spec: &'static CutieSpec) -> CutieStatus {
    let managed_root = managed_dir(app, spec).ok();
    let paths = model_paths(app).ok();
    let installed = paths.as_ref().is_some_and(|paths| {
        paths.all_exist()
            && spec.archive_members.iter().all(|member| {
                let path = paths
                    .encode_key
                    .parent()
                    .unwrap_or(Path::new(""))
                    .join(&member.path);
                crate::models::sha256_file(&path).is_ok_and(|hash| hash == member.sha256)
            })
    });
    let managed = installed
        && paths
            .as_ref()
            .zip(managed_root.as_ref())
            .is_some_and(|(paths, root)| paths.encode_key.parent() == Some(root.as_path()));
    let partial = managed_root
        .as_ref()
        .is_some_and(|root| root.with_extension("zip.part").is_file());
    CutieStatus {
        id: spec.id.clone(),
        name: spec.name.clone(),
        size: spec.size,
        width: spec.width,
        height: spec.height,
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
        provider: "DirectML / CPU",
    }
}

pub fn statuses(app: &AppHandle) -> Vec<CutieStatus> {
    vec![status(app, crate::models::cutie_spec())]
}

fn install_archive(archive: &Path, destination: &Path, spec: &CutieSpec) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or("Cutie model directory is invalid")?;
    let staging = parent.join(format!("{}-installing", spec.destination));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|error| format!("Could not clear Cutie staging folder: {error}"))?;
    }
    std::fs::create_dir_all(&staging)
        .map_err(|error| format!("Could not create Cutie staging folder: {error}"))?;
    let file =
        File::open(archive).map_err(|error| format!("Could not open Cutie archive: {error}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|error| format!("Invalid Cutie archive: {error}"))?;
    for member in &spec.archive_members {
        let expected = &member.path;
        let mut entry = zip
            .by_name(expected)
            .map_err(|_| format!("Cutie archive is missing {expected}"))?;
        if entry.is_dir()
            || entry
                .enclosed_name()
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
                != Some(expected.as_str())
        {
            return Err("Cutie archive contains an unsafe entry".into());
        }
        let output_path = staging.join(expected);
        let mut output = File::create(&output_path)
            .map_err(|error| format!("Could not install Cutie model: {error}"))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Could not extract Cutie model: {error}"))?;
        output.flush().map_err(|error| error.to_string())?;
        if crate::models::sha256_file(&output_path)? != member.sha256 {
            return Err(format!(
                "Cutie archive member {expected} failed checksum verification"
            ));
        }
    }
    if !paths_in(&staging, spec).all_exist() {
        return Err("Cutie installation is incomplete".into());
    }
    if destination.exists() {
        std::fs::remove_dir_all(destination)
            .map_err(|error| format!("Could not replace Cutie: {error}"))?;
    }
    std::fs::rename(&staging, destination)
        .map_err(|error| format!("Could not finish Cutie installation: {error}"))
}

async fn download(
    app: &AppHandle,
    control: &crate::jobs::JobControl,
    archive: &Path,
    spec: &'static CutieSpec,
) -> Result<(), String> {
    let partial = archive.with_extension("zip.part");
    let mut existing = tokio::fs::metadata(&partial)
        .await
        .map(|v| v.len())
        .unwrap_or(0);
    if existing > spec.size {
        let _ = tokio::fs::remove_file(&partial).await;
        existing = 0;
    } else if existing == spec.size {
        if crate::models::sha256_file(&partial)? == spec.sha256 {
            tokio::fs::rename(&partial, archive)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        let _ = tokio::fs::remove_file(&partial).await;
        existing = 0;
    }
    let client = reqwest::Client::builder()
        .user_agent("RotoNow/0.4")
        .build()
        .map_err(|e| e.to_string())?;
    let mut request = client.get(&spec.url);
    if existing > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Cutie download failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Cutie server returned {}", response.status()));
    }
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let start = if resumed { existing } else { 0 };
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&partial)
        .await
        .map_err(|e| e.to_string())?;
    let mut downloaded = start;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if control.cancelled.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        emit_progress(
            app,
            control,
            "downloading",
            Some(downloaded),
            Some(spec.size),
            None,
            format!("Downloading {}", spec.name),
        );
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if crate::models::sha256_file(&partial)? != spec.sha256 {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err("Downloaded Cutie package failed checksum verification".into());
    }
    tokio::fs::rename(&partial, archive)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn download_cutie(
    app: AppHandle,
    jobs: State<'_, JobState>,
    cache: State<'_, crate::cutie::CutieSessionCache>,
) -> Result<String, String> {
    let spec = crate::models::cutie_spec();
    let destination = managed_dir(&app, spec)?;
    let archive = destination.with_extension("zip");
    let control = jobs.begin()?;
    cache.invalidate();
    let id = control.id.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = async {
            if !archive.is_file() || crate::models::sha256_file(&archive)? != spec.sha256 {
                download(&app_for_task, &control, &archive, spec).await?;
            }
            emit_progress(
                &app_for_task,
                &control,
                "installing",
                None,
                None,
                None,
                format!("Installing {}", spec.name),
            );
            let archive_for_install = archive.clone();
            let destination_for_install = destination.clone();
            tauri::async_runtime::spawn_blocking(move || {
                install_archive(&archive_for_install, &destination_for_install, spec)
            })
            .await
            .map_err(|e| e.to_string())?
        }
        .await;
        let event = if control.cancelled.load(Ordering::SeqCst) {
            JobEvent::Cancelled {
                job_id: control.id.clone(),
            }
        } else if let Err(error) = outcome {
            JobEvent::Failed {
                job_id: control.id.clone(),
                error,
            }
        } else {
            JobEvent::Completed {
                job_id: control.id.clone(),
                result: ProcessResult::model_download(&spec.name),
            }
        };
        emit(&app_for_task, event);
        app_for_task.state::<JobState>().finish(&control.id);
    });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_detail_has_pinned_assets_and_stride_aligned_shape() {
        let spec = crate::models::cutie_spec();
        assert_eq!(spec.role, "required");
        assert_eq!(spec.sha256.len(), 64);
        assert_eq!(spec.width % 16, 0);
        assert_eq!(spec.height % 16, 0);
        assert!(spec.size > 100_000_000);
        assert_eq!(files(spec).len(), 4);
        assert!(spec
            .archive_members
            .iter()
            .all(|member| member.sha256.len() == 64));
    }
}
