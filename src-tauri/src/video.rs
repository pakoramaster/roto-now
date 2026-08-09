use crate::{
    inference::{composite_screen, Masker},
    jobs::{emit_progress, JobControl},
    models::ModelId,
    routing::QualityMode,
    temporal::TemporalMaskStabilizer,
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
    sample_aspect_ratio: Option<String>,
    tags: Option<ProbeTags>,
    side_data_list: Option<Vec<ProbeSideData>>,
    disposition: Option<ProbeDisposition>,
}
#[derive(Deserialize)]
struct ProbeTags {
    rotate: Option<String>,
}
#[derive(Deserialize)]
struct ProbeSideData {
    rotation: Option<f64>,
}
#[derive(Deserialize)]
struct ProbeDisposition {
    attached_pic: Option<u8>,
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
    has_audio: bool,
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

pub fn routing_sample(app: &AppHandle, input: &Path) -> Result<DynamicImage, String> {
    let ffmpeg = bundled_binary(app, "ffmpeg")?;
    let output = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=192:192:force_original_aspect_ratio=decrease",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "pipe:1",
        ])
        .output()
        .map_err(|error| format!("Could not sample the video for automatic routing: {error}"))?;
    if !output.status.success() {
        return Err("Could not sample this video for automatic routing".into());
    }
    image::load_from_memory(&output.stdout)
        .map_err(|error| format!("Could not read the automatic-routing frame: {error}"))
}

fn parse_rate(rate: &str) -> Option<f64> {
    let (a, b) = rate.split_once('/').or_else(|| rate.split_once(':'))?;
    let numerator: f64 = a.parse().ok()?;
    let denominator: f64 = b.parse().ok()?;
    let value = numerator / denominator;
    (denominator > 0.0 && value.is_finite() && (0.1..=240.0).contains(&value)).then_some(value)
}

fn select_frame_rate(average: Option<&str>, nominal: Option<&str>) -> (String, f64) {
    for candidate in [average, nominal].into_iter().flatten() {
        if let Some(value) = parse_rate(candidate) {
            return (candidate.to_string(), value);
        }
    }
    ("30/1".into(), 30.0)
}

fn parse_aspect_ratio(value: Option<&str>) -> f64 {
    value.and_then(parse_rate).unwrap_or(1.0)
}

fn normalized_rotation(video: &ProbeStream) -> i32 {
    let raw = video
        .side_data_list
        .as_deref()
        .and_then(|items| items.iter().find_map(|item| item.rotation))
        .or_else(|| video.tags.as_ref()?.rotate.as_deref()?.parse().ok())
        .unwrap_or(0.0);
    let rounded = (raw / 90.0).round() as i32 * 90;
    ((rounded % 360) + 360) % 360
}

fn even(value: f64) -> u32 {
    ((value.round().max(2.0) as u32) / 2) * 2
}

fn normalized_dimensions(width: u32, height: u32, sample_aspect: f64, rotation: i32) -> (u32, u32) {
    let square_width = even(width as f64 * sample_aspect);
    let square_height = even(height as f64);
    if matches!(rotation, 90 | 270) {
        (square_height, square_width)
    } else {
        (square_width, square_height)
    }
}

fn encoding_profile(quality: QualityMode) -> (&'static str, &'static str) {
    match quality {
        QualityMode::Fast => ("23", "veryfast"),
        QualityMode::Balanced => ("18", "medium"),
        QualityMode::Maximum => ("16", "slow"),
    }
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
        .find(|stream| {
            stream.codec_type.as_deref() == Some("video")
                && stream
                    .disposition
                    .as_ref()
                    .and_then(|value| value.attached_pic)
                    != Some(1)
        })
        .ok_or("The file has no video stream")?;
    let coded_width = video.width.ok_or("Video width is unavailable")?;
    let coded_height = video.height.ok_or("Video height is unavailable")?;
    let rotation = normalized_rotation(video);
    let (width, height) = normalized_dimensions(
        coded_width,
        coded_height,
        parse_aspect_ratio(video.sample_aspect_ratio.as_deref()),
        rotation,
    );
    let (fps_arg, fps) = select_frame_rate(
        video.avg_frame_rate.as_deref(),
        video.r_frame_rate.as_deref(),
    );
    let stream_duration = video
        .duration
        .as_deref()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0);
    let reported_frames = video
        .nb_frames
        .as_deref()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0_u64);
    let format_duration = value
        .format
        .as_ref()
        .and_then(|format| format.duration.as_deref())
        .and_then(|duration| duration.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0);
    let duration = stream_duration
        .or_else(|| (reported_frames > 0).then_some(reported_frames as f64 / fps))
        .or(format_duration)
        .ok_or("Video duration is unavailable")?;
    let frames = (duration * fps).round().max(1.0) as u64;
    Ok(VideoMeta {
        width,
        height,
        fps_arg,
        duration,
        frames,
        has_audio: value
            .streams
            .iter()
            .any(|stream| stream.codec_type.as_deref() == Some("audio")),
    })
}

fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    let scale = (1280.0 / width as f64).min(720.0 / height as f64).min(1.0);
    (even(width as f64 * scale), even(height as f64 * scale))
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn child_error(child: &mut Child) -> String {
    let mut message = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut message);
    }
    message.trim().chars().take(1200).collect()
}

fn validate_export(
    ffprobe: &Path,
    output: &Path,
    expected_width: u32,
    expected_height: u32,
    expected_duration: f64,
    expected_audio: bool,
    fps: f64,
) -> Result<(), String> {
    let result = probe(ffprobe, output)
        .map_err(|error| format!("The encoded MP4 could not be validated: {error}"))?;
    if result.width != expected_width || result.height != expected_height {
        return Err(format!(
            "The encoded MP4 has unexpected dimensions ({}x{} instead of {expected_width}x{expected_height})",
            result.width, result.height
        ));
    }
    let tolerance = (0.6 / fps.max(1.0)).max(0.08);
    if (result.duration - expected_duration).abs() > tolerance {
        return Err(format!(
            "The encoded MP4 duration drifted by {:.3} seconds",
            result.duration - expected_duration
        ));
    }
    if expected_audio && !result.has_audio {
        return Err("The encoded MP4 is missing the source audio".into());
    }
    Ok(())
}

pub struct VideoOutcome {
    pub frame_count: u64,
    pub provider: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub duration: f64,
    pub has_audio: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn process_video(
    app: &AppHandle,
    control: &JobControl,
    input: &Path,
    output: &Path,
    model_id: ModelId,
    edge_detail: u8,
    quality: &str,
    screen_color: &str,
    preview: bool,
    start_seconds: f64,
) -> Result<VideoOutcome, String> {
    let ffmpeg = bundled_binary(app, "ffmpeg")?;
    let ffprobe = bundled_binary(app, "ffprobe")?;
    let model_path = crate::models::model_path(app, model_id)?;
    app.state::<crate::inference::ModelSessionCache>()
        .with_model(
            model_path,
            model_id,
            model_id != ModelId::General,
            || {
                emit_progress(
                    app,
                    control,
                    "loadingModel",
                    None,
                    None,
                    None,
                    "Loading segmentation model",
                )
            },
            |masker, reused| {
                if reused {
                    emit_progress(
                        app,
                        control,
                        "loadingModel",
                        None,
                        None,
                        None,
                        "Using loaded segmentation model",
                    );
                }
                process_video_with_masker(
                    Some(app),
                    control,
                    input,
                    output,
                    masker,
                    &ffmpeg,
                    &ffprobe,
                    edge_detail,
                    quality,
                    screen_color,
                    preview,
                    start_seconds,
                )
            },
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
    quality: &str,
    screen_color: &str,
    preview: bool,
    start_seconds: f64,
) -> Result<VideoOutcome, String> {
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
    process_video_with_masker(
        app,
        control,
        input,
        output,
        &mut masker,
        ffmpeg,
        ffprobe,
        edge_detail,
        quality,
        screen_color,
        preview,
        start_seconds,
    )
}

#[allow(clippy::too_many_arguments)]
fn process_video_with_masker(
    app: Option<&AppHandle>,
    control: &JobControl,
    input: &Path,
    output: &Path,
    masker: &mut Masker,
    ffmpeg: &Path,
    ffprobe: &Path,
    edge_detail: u8,
    quality: &str,
    screen_color: &str,
    preview: bool,
    start_seconds: f64,
) -> Result<VideoOutcome, String> {
    let quality_mode = QualityMode::parse(quality)?;
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
            "12/1".to_string(),
            (clip_duration * 12.0).ceil() as u64,
        )
    } else {
        (meta.width, meta.height, meta.fps_arg.clone(), meta.frames)
    };
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
    let decode_filter = if preview {
        format!("fps=12,scale={width}:{height}:flags=lanczos,setsar=1")
    } else {
        format!(
            "fps={},scale={width}:{height}:flags=lanczos,setsar=1",
            meta.fps_arg
        )
    };
    decode_args.extend([
        "-vf".into(),
        decode_filter,
        "-frames:v".into(),
        total.to_string(),
    ]);
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
        "-fflags".into(),
        "+genpts".into(),
        "-s".into(),
        format!("{width}x{height}"),
        "-framerate".into(),
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
    let (full_crf, full_preset) = encoding_profile(quality_mode);
    encode_args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    encode_args.extend(["-map".into(), "0:v:0".into()]);
    if meta.has_audio {
        encode_args.extend(["-map".into(), "1:a:0?".into()]);
    } else {
        encode_args.push("-an".into());
    }
    encode_args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-crf".into(),
        if preview {
            "24".into()
        } else {
            full_crf.into()
        },
        "-preset".into(),
        if preview {
            "veryfast".into()
        } else {
            full_preset.into()
        },
        "-fps_mode".into(),
        "cfr".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-metadata:s:v:0".into(),
        "rotate=0".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    if meta.has_audio {
        encode_args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            "-af".into(),
            format!(
                "atrim=duration={clip_duration:.6},asetpts=PTS-STARTPTS,aresample=async=1000:first_pts=0"
            ),
        ]);
    }
    encode_args.push(output.to_string_lossy().into_owned());
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
    let mut stabilizer = TemporalMaskStabilizer::default();
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
        let mut cutout = match masker.apply(
            &DynamicImage::ImageRgb8(image),
            edge_detail,
            quality,
            control,
        ) {
            Ok(cutout) => cutout,
            Err(error) => break Err(error),
        };
        if let Err(error) = stabilizer.apply(&bytes, &mut cutout) {
            break Err(error);
        }
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
        let decoder_error = child_error(&mut decoder);
        let encoder_error = child_error(&mut encoder);
        let _ = std::fs::remove_file(output);
        let details = [decoder_error, encoder_error]
            .into_iter()
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(if details.is_empty() {
            "FFmpeg could not finish the video export".into()
        } else {
            format!("FFmpeg could not finish the video export: {details}")
        });
    }
    let output_fps = parse_rate(&fps_arg).unwrap_or(30.0);
    if let Err(error) = validate_export(
        ffprobe,
        output,
        width,
        height,
        clip_duration,
        meta.has_audio,
        output_fps,
    ) {
        let _ = std::fs::remove_file(output);
        return Err(error);
    }
    Ok(VideoOutcome {
        frame_count,
        provider: masker.provider().into(),
        width,
        height,
        frame_rate: output_fps,
        duration: clip_duration,
        has_audio: meta.has_audio,
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
        assert!(parse_rate("1000/1").is_none());
        assert_eq!(parse_aspect_ratio(Some("2:1")), 2.0);
    }

    #[test]
    fn invalid_average_rate_falls_back_to_nominal_rate() {
        let (argument, value) = select_frame_rate(Some("0/0"), Some("24000/1001"));
        assert_eq!(argument, "24000/1001");
        assert!((value - 23.976).abs() < 0.001);
        assert_eq!(select_frame_rate(Some("bad"), None), ("30/1".into(), 30.0));
    }

    #[test]
    fn display_dimensions_normalize_rotation_aspect_and_odd_sizes() {
        assert_eq!(normalized_dimensions(1921, 1081, 1.0, 0), (1920, 1080));
        assert_eq!(normalized_dimensions(320, 214, 2.0, 90), (214, 640));
    }

    #[test]
    fn display_matrix_rotation_is_normalized() {
        let stream = ProbeStream {
            codec_type: Some("video".into()),
            width: Some(1920),
            height: Some(1080),
            avg_frame_rate: Some("30/1".into()),
            r_frame_rate: Some("30/1".into()),
            duration: Some("1".into()),
            nb_frames: Some("30".into()),
            sample_aspect_ratio: Some("1:1".into()),
            tags: None,
            side_data_list: Some(vec![ProbeSideData {
                rotation: Some(-90.0),
            }]),
            disposition: None,
        };
        assert_eq!(normalized_rotation(&stream), 270);
    }

    #[test]
    fn preview_duration_is_one_second() {
        assert_eq!(PREVIEW_SECONDS, 1.0);
    }

    #[test]
    fn quality_modes_have_distinct_video_profiles() {
        assert_eq!(encoding_profile(QualityMode::Fast), ("23", "veryfast"));
        assert_eq!(encoding_profile(QualityMode::Balanced), ("18", "medium"));
        assert_eq!(encoding_profile(QualityMode::Maximum), ("16", "slow"));
    }
}
