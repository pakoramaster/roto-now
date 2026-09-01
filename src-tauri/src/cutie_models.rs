use crate::{
    cutie::CutieModelPaths,
    jobs::{emit, emit_progress, JobEvent, JobState, ProcessResult},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
use tauri::{AppHandle, Manager, State};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CutieTier {
    Balanced,
    High,
}

impl CutieTier {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "balanced" => Ok(Self::Balanced),
            "high" => Ok(Self::High),
            _ => Err("Unknown Cutie quality tier".into()),
        }
    }
}

struct CutieSpec {
    tier: CutieTier,
    id: &'static str,
    name: &'static str,
    folder: &'static str,
    asset: &'static str,
    sha256: &'static str,
    size: u64,
    width: usize,
    height: usize,
}

const BASE_URL: &str = "https://github.com/OpenShot/openshot-onnx/releases/download/v0.2.0";
const SPECS: [CutieSpec; 2] = [
    CutieSpec {
        tier: CutieTier::Balanced,
        id: "cutieBalanced",
        name: "Cutie Balanced",
        folder: "cutie-medium",
        asset: "cutie-opencv-medium-640x368.zip",
        sha256: "64f79a30d4e53f2aad597772968f18dcc3806ef8dfbe1fefcbf4b58fac069709",
        size: 130_823_166,
        width: 640,
        height: 368,
    },
    CutieSpec {
        tier: CutieTier::High,
        id: "cutieHigh",
        name: "Cutie High Detail",
        folder: "cutie-high",
        asset: "cutie-opencv-high-960x544.zip",
        sha256: "56c5b4823610c8f87b551b82893ef0450f900c254b4ff729f24aec30dea2124f",
        size: 131_949_289,
        width: 960,
        height: 544,
    },
];

fn spec(tier: CutieTier) -> &'static CutieSpec {
    SPECS
        .iter()
        .find(|item| item.tier == tier)
        .expect("Cutie tier registry is complete")
}

fn files(spec: &CutieSpec) -> [String; 4] {
    let size = format!("{}x{}", spec.width, spec.height);
    [
        format!("cutie-encode-key-{size}.onnx"),
        format!("cutie-encode-value-{size}.onnx"),
        format!("cutie-memory-readout-floatmask-valid-{size}-m6-topk30-opencv.onnx"),
        format!("cutie-decode-{size}.onnx"),
    ]
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutieStatus {
    pub id: &'static str,
    pub tier: CutieTier,
    pub name: &'static str,
    pub size: u64,
    pub width: usize,
    pub height: usize,
    pub installed: bool,
    pub managed: bool,
    pub state: &'static str,
    pub provider: &'static str,
}

fn managed_dir(app: &AppHandle, spec: &CutieSpec) -> Result<PathBuf, String> {
    Ok(crate::models::model_root(app)?.join(spec.folder))
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

pub fn model_paths(app: &AppHandle, tier: CutieTier) -> Result<CutieModelPaths, String> {
    let spec = spec(tier);
    let managed = paths_in(&managed_dir(app, spec)?, spec);
    if managed.all_exist() {
        return Ok(managed);
    }
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("ROTO_NOW_MODEL_ROOT") {
        let local = paths_in(&PathBuf::from(root).join(spec.folder), spec);
        if local.all_exist() {
            return Ok(local);
        }
    }
    Ok(managed)
}

fn status(app: &AppHandle, spec: &'static CutieSpec) -> CutieStatus {
    let managed_root = managed_dir(app, spec).ok();
    let paths = model_paths(app, spec.tier).ok();
    let installed = paths.as_ref().is_some_and(CutieModelPaths::all_exist);
    let managed = installed
        && paths
            .as_ref()
            .zip(managed_root.as_ref())
            .is_some_and(|(paths, root)| paths.encode_key.parent() == Some(root.as_path()));
    let partial = managed_root
        .as_ref()
        .is_some_and(|root| root.with_extension("zip.part").is_file());
    CutieStatus {
        id: spec.id,
        tier: spec.tier,
        name: spec.name,
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
    SPECS.iter().map(|item| status(app, item)).collect()
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("Could not verify Cutie: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn install_archive(archive: &Path, destination: &Path, spec: &CutieSpec) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or("Cutie model directory is invalid")?;
    let staging = parent.join(format!("{}-installing", spec.folder));
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
    for expected in files(spec) {
        let mut entry = zip
            .by_name(&expected)
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
        let mut output = File::create(staging.join(&expected))
            .map_err(|error| format!("Could not install Cutie model: {error}"))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Could not extract Cutie model: {error}"))?;
        output.flush().map_err(|error| error.to_string())?;
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
        if sha256(&partial)? == spec.sha256 {
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
    let mut request = client.get(format!("{BASE_URL}/{}", spec.asset));
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
    if sha256(&partial)? != spec.sha256 {
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
    tier: String,
) -> Result<String, String> {
    let spec = spec(CutieTier::parse(&tier)?);
    let destination = managed_dir(&app, spec)?;
    let archive = destination.with_extension("zip");
    let control = jobs.begin()?;
    cache.invalidate();
    let id = control.id.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = async {
            if !archive.is_file() || sha256(&archive)? != spec.sha256 {
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
                result: ProcessResult::model_download(spec.name),
            }
        };
        emit(&app_for_task, event);
        app_for_task.state::<JobState>().finish(&control.id);
    });
    Ok(id)
}

#[tauri::command]
pub fn remove_cutie(
    app: AppHandle,
    cache: State<'_, crate::cutie::CutieSessionCache>,
    tier: String,
) -> Result<(), String> {
    let spec = spec(CutieTier::parse(&tier)?);
    cache.invalidate();
    let destination = managed_dir(&app, spec)?;
    if destination.exists() {
        std::fs::remove_dir_all(&destination)
            .map_err(|e| format!("Could not remove {}: {e}", spec.name))?;
    }
    for path in [
        destination.with_extension("zip"),
        destination.with_extension("zip.part"),
    ] {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_have_pinned_archives_and_stride_aligned_shapes() {
        for item in SPECS.iter() {
            assert_eq!(item.sha256.len(), 64);
            assert_eq!(item.width % 16, 0);
            assert_eq!(item.height % 16, 0);
            assert!(item.size > 100_000_000);
            assert_eq!(files(item).len(), 4);
        }
    }
}
