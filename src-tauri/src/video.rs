use crate::{
    inference::{composite_screen, Masker},
    jobs::{emit_progress, JobControl},
    models::ModelId,
};
use image::{DynamicImage, ImageBuffer, Rgb};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::Ordering,
    time::Instant,
};
use tauri::{AppHandle, Manager};

const PREVIEW_SECONDS: f64 = 1.0;

#[derive(Deserialize)]
struct Probe {
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}
#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    nb_frames: Option<String>,
}
#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

struct VideoMeta {
    width: u32,
    height: u32,
    fps_arg: String,
    duration: f64,
    frames: u64,
}

fn developer_binary(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("bin")
        .join(format!("{name}.exe"))
}

fn bundled_binary(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    let packaged = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?
        .join("bin")
        .join(format!("{name}.exe"));
    if packaged.is_file() {
        return Ok(packaged);
    }
    let development = developer_binary(name);
    if development.is_file() {
        return Ok(development);
    }
    Err(format!(
        "Bundled {name}.exe is missing. Run scripts/fetch-ffmpeg.ps1 before building."
    ))
}

fn parse_rate(rate: &str) -> Option<f64> {
    let (a, b) = rate.split_once('/')?;
    let numerator: f64 = a.parse().ok()?;
    let denominator: f64 = b.parse().ok()?;
    (denominator > 0.0).then_some(numerator / denominator)
}

fn probe(ffprobe: &Path, source: &Path) -> Result<VideoMeta, String> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(source)
        .output()
        .map_err(|error| format!("Could not start FFprobe: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "FFprobe could not read this video: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let value: Probe = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("FFprobe returned invalid metadata: {error}"))?;
    let video = value
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or("The file has no video stream")?;
    let width = video.width.ok_or("Video width is unavailable")?;
    let height = video.height.ok_or("Video height is unavailable")?;
    let fps_arg = video
        .avg_frame_rate
        .as_deref()
        .filter(|v| *v != "0/0")
        .or(video.r_frame_rate.as_deref())
        .unwrap_or("30/1")
        .to_string();
    let fps = parse_rate(&fps_arg).unwrap_or(30.0);
    let duration = video
        .duration
        .as_deref()
        .and_then(|v| v.parse().ok())
        .or_else(|| value.format.as_ref()?.duration.as_deref()?.parse().ok())
        .unwrap_or(0.0);
    let frames = video
        .nb_frames
        .as_deref()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| (duration * fps).ceil() as u64);
    Ok(VideoMeta {
        width,
        height,
        fps_arg,
        duration,
        frames,
    })
}

fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    let scale = (1280.0 / width as f64).min(720.0 / height as f64).min(1.0);
    let even = |value: f64| (((value.floor() as u32).max(2)) / 2) * 2;
    (even(width as f64 * scale), even(height as f64 * scale))
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub struct VideoOutcome {
    pub frame_count: u64,
    pub provider: String,
}

#[allow(clippy::too_many_arguments)]
pub fn process_video(
    app: &AppHandle,
    control: &JobControl,
    input: &Path,
    output: &Path,
    model_id: ModelId,
    edge_detail: u8,
    screen_color: &str,
    preview: bool,
    start_seconds: f64,
) -> Result<VideoOutcome, String> {
    let ffmpeg = bundled_binary(app, "ffmpeg")?;
    let ffprobe = bundled_binary(app, "ffprobe")?;
    let model_path = crate::models::model_path(app, model_id)?;
    process_video_with_paths(
        Some(app),
        control,
        input,
        output,
        model_id,
        &model_path,
        &ffmpeg,
        &ffprobe,
        edge_detail,
        screen_color,
        preview,
        start_seconds,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn process_video_with_paths(
    app: Option<&AppHandle>,
    control: &JobControl,
    input: &Path,
    output: &Path,
    model_id: ModelId,
    model_path: &Path,
    ffmpeg: &Path,
    ffprobe: &Path,
    edge_detail: u8,
    screen_color: &str,
    preview: bool,
    start_seconds: f64,
) -> Result<VideoOutcome, String> {
    let meta = probe(ffprobe, input)?;
    let start = start_seconds.max(0.0).min(meta.duration.max(0.0));
    let clip_duration = if preview {
        (meta.duration - start).clamp(0.0, PREVIEW_SECONDS)
    } else {
        meta.duration
    };
    if preview && clip_duration <= 0.0 {
        return Err("The playhead is at the end of the video".into());
    }
    let (width, height, fps_arg, total) = if preview {
        let (width, height) = preview_dimensions(meta.width, meta.height);
        (
            width,
            height,
            "12".to_string(),
            (clip_duration * 12.0).ceil() as u64,
        )
    } else {
        (meta.width, meta.height, meta.fps_arg.clone(), meta.frames)
    };

    if let Some(app) = app {
        emit_progress(
            app,
            control,
            "loadingModel",
            None,
            None,
            None,
            "Loading segmentation model",
        );
    }
    let mut masker = Masker::load_from_path(
        model_path.to_path_buf(),
        model_id,
        model_id != ModelId::General,
    )?;
    if control.cancelled.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }

    let mut decode_args = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    if preview {
        decode_args.extend([
            "-ss".into(),
            format!("{start:.3}"),
            "-t".into(),
            format!("{clip_duration:.3}"),
        ]);
    }
    decode_args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    if preview {
        decode_args.extend([
            "-vf".into(),
            format!("fps=12,scale={width}:{height}:flags=lanczos"),
        ]);
    }
    decode_args.extend([
        "-an".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "pipe:1".into(),
    ]);
    let mut decoder = Command::new(ffmpeg)
        .args(&decode_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start video decoder: {error}"))?;

    let mut encode_args = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-s".into(),
        format!("{width}x{height}"),
        "-r".into(),
        fps_arg.clone(),
        "-i".into(),
        "pipe:0".into(),
    ];
    if preview {
        encode_args.extend([
            "-ss".into(),
            format!("{start:.3}"),
            "-t".into(),
            format!("{clip_duration:.3}"),
        ]);
    }
    encode_args.extend([
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "1:a?".into(),
        "-c:v".into(),
        "libx264".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-crf".into(),
        if preview { "24".into() } else { "18".into() },
        "-preset".into(),
        if preview {
            "veryfast".into()
        } else {
            "medium".into()
        },
        "-c:a".into(),
        "aac".into(),
        "-shortest".into(),
        output.to_string_lossy().into_owned(),
    ]);
    let mut encoder = Command::new(ffmpeg)
        .args(&encode_args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            kill(&mut decoder);
            format!("Could not start video encoder: {error}")
        })?;

    let mut reader = match decoder.stdout.take() {
        Some(reader) => reader,
        None => {
            kill(&mut decoder);
            kill(&mut encoder);
            let _ = std::fs::remove_file(output);
            return Err("Could not open decoder pipe".into());
        }
    };
    let mut writer = match encoder.stdin.take() {
        Some(writer) => writer,
        None => {
            kill(&mut decoder);
            kill(&mut encoder);
            let _ = std::fs::remove_file(output);
            return Err("Could not open encoder pipe".into());
        }
    };
    let frame_size = width as usize * height as usize * 3;
    let mut bytes = vec![0_u8; frame_size];
    let mut frame_count = 0_u64;
    let mut ewma: Option<f64> = None;
    let result = loop {
        if control.cancelled.load(Ordering::SeqCst) {
            break Err("cancelled".into());
        }
        match reader.read_exact(&mut bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break Ok(()),
            Err(error) => break Err(format!("Could not decode video frame: {error}")),
        }
        let frame_started = Instant::now();
        let image = match ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, bytes.clone()) {
            Some(image) => image,
            None => break Err("Could not construct video frame".into()),
        };
        let cutout = match masker.apply(&DynamicImage::ImageRgb8(image), edge_detail, control) {
            Ok(cutout) => cutout,
            Err(error) => break Err(error),
        };
        let composited = composite_screen(&cutout, screen_color);
        if let Err(error) = writer.write_all(&composited) {
            break Err(format!("Could not encode video frame: {error}"));
        }
        frame_count += 1;
        let seconds = frame_started.elapsed().as_secs_f64();
        ewma = Some(ewma.map(|old| old * 0.8 + seconds * 0.2).unwrap_or(seconds));
        let eta = (frame_count >= 3 && total > frame_count)
            .then(|| (ewma.unwrap_or(0.0) * (total - frame_count) as f64).ceil() as u64);
        if let Some(app) = app {
            emit_progress(
                app,
                control,
                "processingFrames",
                Some(frame_count),
                Some(total.max(frame_count)),
                eta,
                format!("Processing frame {frame_count} of {total}"),
            );
        }
    };
    drop(writer);
    if let Err(error) = result {
        kill(&mut decoder);
        kill(&mut encoder);
        let _ = std::fs::remove_file(output);
        return Err(error);
    }
    let decoder_status = match decoder.wait() {
        Ok(status) => status,
        Err(error) => {
            kill(&mut encoder);
            let _ = std::fs::remove_file(output);
            return Err(format!("Could not finish decoder: {error}"));
        }
    };
    let encoder_status = match encoder.wait() {
        Ok(status) => status,
        Err(error) => {
            let _ = std::fs::remove_file(output);
            return Err(format!("Could not finish encoder: {error}"));
        }
    };
    if control.cancelled.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(output);
        return Err("cancelled".into());
    }
    if !decoder_status.success() || !encoder_status.success() {
        let _ = std::fs::remove_file(output);
        return Err("FFmpeg could not finish the video export".into());
    }
    Ok(VideoOutcome {
        frame_count,
        provider: masker.provider().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_size_is_even_and_bounded() {
        for (source_width, source_height) in [(1920, 1080), (1080, 1920), (1279, 719), (321, 215)] {
            let (width, height) = preview_dimensions(source_width, source_height);
            assert!(width <= 1280 && height <= 720);
            assert_eq!(width % 2, 0);
            assert_eq!(height % 2, 0);
        }
    }

    #[test]
    fn rational_frame_rates_are_parsed() {
        assert_eq!(parse_rate("30000/1001").unwrap().round(), 30.0);
        assert!(parse_rate("0/0").is_none());
    }

    #[test]
    fn preview_duration_is_one_second() {
        assert_eq!(PREVIEW_SECONDS, 1.0);
    }
}
