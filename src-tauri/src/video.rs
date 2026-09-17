use crate::{
    inference::{composite_screen, prepare_video_frame, save_cutout, Masker, PreparedFrame},
    jobs::{emit_progress, JobControl, PerformanceMetrics},
    models::ModelId,
    routing::QualityMode,
    temporal::TemporalMaskStabilizer,
};
use image::{DynamicImage, GrayImage, ImageBuffer, Luma, Rgb};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PREVIEW_FRAME_COUNT: u64 = 1;

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
    color_range: Option<String>,
    color_space: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
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
    color: ColorSpec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ColorSpec {
    range: String,
    space: String,
    transfer: String,
    primaries: String,
    filter_matrix: String,
}

fn supported_color_value(value: Option<&str>, supported: &[&str], fallback: &str) -> String {
    value
        .filter(|candidate| supported.contains(candidate))
        .unwrap_or(fallback)
        .to_string()
}

fn color_spec(video: &ProbeStream, width: u32, height: u32) -> ColorSpec {
    // When an SDR source is untagged, use the conventional SD/HD defaults.
    // Explicit tags always win, and are written back to the encoded stream.
    let hd = width >= 1280 || height > 576;
    let default_space = if hd { "bt709" } else { "smpte170m" };
    let space = supported_color_value(
        video.color_space.as_deref(),
        &[
            "bt709",
            "fcc",
            "bt470bg",
            "smpte170m",
            "smpte240m",
            "bt2020nc",
            "bt2020c",
        ],
        default_space,
    );
    let filter_matrix = match space.as_str() {
        "bt470bg" | "smpte170m" => "bt601",
        "bt2020nc" | "bt2020c" => "bt2020",
        value => value,
    }
    .to_string();
    ColorSpec {
        range: supported_color_value(video.color_range.as_deref(), &["tv", "pc"], "tv"),
        transfer: supported_color_value(
            video.color_transfer.as_deref(),
            &[
                "bt709",
                "gamma22",
                "gamma28",
                "smpte170m",
                "smpte240m",
                "linear",
                "iec61966-2-4",
                "bt1361e",
                "iec61966-2-1",
                "bt2020-10",
                "bt2020-12",
            ],
            "bt709",
        ),
        primaries: supported_color_value(
            video.color_primaries.as_deref(),
            &[
                "bt709",
                "bt470m",
                "bt470bg",
                "smpte170m",
                "smpte240m",
                "film",
                "bt2020",
                "smpte428",
                "smpte431",
                "smpte432",
            ],
            if hd { "bt709" } else { "smpte170m" },
        ),
        space,
        filter_matrix,
    }
}

fn scale_to_rgb_filter(width: u32, height: u32, color: &ColorSpec) -> String {
    format!(
        "scale={width}:{height}:flags=lanczos:in_range={}:out_range=pc:in_color_matrix={},setsar=1",
        color.range, color.filter_matrix
    )
}

fn rgb_to_yuv_filter(color: &ColorSpec) -> String {
    format!(
        "scale=in_range=pc:out_range={}:out_color_matrix={},format=yuv420p",
        color.range, color.filter_matrix
    )
}

fn append_color_args(args: &mut Vec<String>, color: &ColorSpec, encoder: VideoEncoder) {
    args.extend([
        "-color_range".into(),
        color.range.clone(),
        "-colorspace".into(),
        color.space.clone(),
        "-color_trc".into(),
        color.transfer.clone(),
        "-color_primaries".into(),
        color.primaries.clone(),
    ]);
    if encoder == VideoEncoder::Software {
        // FFmpeg's generic color options do not currently propagate primaries
        // and transfer characteristics into libx264's H.264 VUI. Supply the
        // equivalent x264 parameters so players receive all four properties.
        let transfer = match color.transfer.as_str() {
            "gamma22" => "bt470m",
            "gamma28" => "bt470bg",
            value => value,
        };
        args.extend([
            "-x264-params".into(),
            format!(
                "colorprim={}:transfer={transfer}:colormatrix={}:fullrange={}",
                color.primaries,
                color.space,
                if color.range == "pc" { "on" } else { "off" }
            ),
        ]);
    }
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

fn background_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
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
        QualityMode::Balanced => ("19", "fast"),
        QualityMode::Maximum => ("16", "slow"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VideoEncoder {
    Nvidia,
    Software,
}

impl VideoEncoder {
    fn label(self) -> &'static str {
        match self {
            Self::Nvidia => "NVIDIA NVENC",
            Self::Software => "software H.264",
        }
    }
}

fn select_video_encoder(ffmpeg: &Path, width: u32, height: u32) -> VideoEncoder {
    // Listing encoders only proves that FFmpeg was compiled with NVENC. Encode
    // one frame at the output dimensions so missing drivers, unsupported
    // dimensions, and current VRAM pressure all fall back before export.
    let probe_source = format!("color=c=black:s={width}x{height}:r=1");
    let status = background_command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &probe_source,
            "-frames:v",
            "1",
            "-an",
            "-c:v",
            "h264_nvenc",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "null",
            "-",
        ])
        .status();
    if status.is_ok_and(|status| status.success()) {
        VideoEncoder::Nvidia
    } else {
        VideoEncoder::Software
    }
}

fn append_video_encoder_args(args: &mut Vec<String>, quality: QualityMode, encoder: VideoEncoder) {
    let (quality_value, preset) = encoding_profile(quality);
    match encoder {
        VideoEncoder::Nvidia => {
            let preset = match quality {
                QualityMode::Fast => "p2",
                QualityMode::Balanced => "p4",
                QualityMode::Maximum => "p6",
            };
            args.extend([
                "-c:v".into(),
                "h264_nvenc".into(),
                "-preset".into(),
                preset.into(),
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                quality_value.into(),
                "-b:v".into(),
                "0".into(),
            ]);
        }
        VideoEncoder::Software => args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-crf".into(),
            quality_value.into(),
            "-preset".into(),
            preset.into(),
        ]),
    }
}

pub(crate) fn validate_video_input(app: &AppHandle, source: &Path) -> Result<(), String> {
    let ffprobe = bundled_binary(app, "ffprobe")?;
    probe(&ffprobe, source).map(|_| ())
}

fn probe(ffprobe: &Path, source: &Path) -> Result<VideoMeta, String> {
    let output = background_command(ffprobe)
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
    crate::media_limits::video_frame_bytes(coded_width, coded_height)?;
    let rotation = normalized_rotation(video);
    let (width, height) = normalized_dimensions(
        coded_width,
        coded_height,
        parse_aspect_ratio(video.sample_aspect_ratio.as_deref()),
        rotation,
    );
    crate::media_limits::video_frame_bytes(width, height)?;
    let color = color_spec(video, width, height);
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
        color,
    })
}

fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    let scale = (1280.0 / width as f64).min(720.0 / height as f64).min(1.0);
    (even(width as f64 * scale), even(height as f64 * scale))
}

const STDERR_TAIL_BYTES: usize = 64 * 1024;

fn drain_stderr(mut reader: impl Read) -> Vec<u8> {
    let mut tail = VecDeque::with_capacity(STDERR_TAIL_BYTES);
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let excess = (tail.len() + count).saturating_sub(STDERR_TAIL_BYTES);
                tail.drain(..excess);
                tail.extend(&buffer[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    tail.into_iter().collect()
}

// Own both the process and its stderr worker so every early return reaps them.
struct FfmpegChild {
    child: Child,
    stderr_worker: Option<thread::JoinHandle<Vec<u8>>>,
    stderr_tail: Vec<u8>,
}

impl FfmpegChild {
    fn new(mut child: Child) -> Self {
        let stderr_worker = child
            .stderr
            .take()
            .map(|stderr| thread::spawn(move || drain_stderr(stderr)));
        Self {
            child,
            stderr_worker,
            stderr_tail: Vec::new(),
        }
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait()?;
        self.join_stderr();
        Ok(status)
    }

    fn join_stderr(&mut self) {
        if let Some(worker) = self.stderr_worker.take() {
            self.stderr_tail = worker.join().unwrap_or_default();
        }
    }
}

impl std::ops::Deref for FfmpegChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}

impl std::ops::DerefMut for FfmpegChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for FfmpegChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.join_stderr();
    }
}

fn kill(child: &mut FfmpegChild) {
    let _ = child.child.kill();
    let _ = child.wait();
}

fn child_error(child: &mut FfmpegChild) -> String {
    String::from_utf8_lossy(&child.stderr_tail)
        .trim()
        .chars()
        .take(1200)
        .collect()
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
    pub precision: String,
    pub pipeline: String,
    pub performance: PerformanceMetrics,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub duration: f64,
    pub has_audio: bool,
}

pub struct SeedOutcome {
    pub provider: String,
    pub precision: String,
    pub width: u32,
    pub height: u32,
    pub performance: PerformanceMetrics,
}

fn prepare_imported_seed(
    frame: &ImageBuffer<Rgb<u8>, Vec<u8>>,
    supplied: DynamicImage,
) -> Result<(image::RgbaImage, &'static str), String> {
    let source_ratio = frame.width() as f64 / frame.height().max(1) as f64;
    let mask_ratio = supplied.width() as f64 / supplied.height().max(1) as f64;
    if (mask_ratio / source_ratio - 1.0).abs() > 0.02 {
        return Err("The attached mask must have the same aspect ratio as the video frame".into());
    }
    let rgba = supplied.to_rgba8();
    let has_transparency = rgba.pixels().any(|pixel| pixel[3] < 255);
    let alpha = if has_transparency {
        GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            Luma([rgba.get_pixel(x, y)[3]])
        })
    } else {
        let luminance = DynamicImage::ImageRgba8(rgba).to_luma8();
        let (minimum, maximum) = luminance
            .pixels()
            .fold((u8::MAX, u8::MIN), |(low, high), pixel| {
                (low.min(pixel[0]), high.max(pixel[0]))
            });
        if (minimum > 64 || maximum < 191) && minimum < 128 && maximum >= 128 {
            return Err(
                "An opaque mask PNG must use white for the subject and black for the background"
                    .into(),
            );
        }
        luminance
    };
    let alpha = if alpha.dimensions() == frame.dimensions() {
        alpha
    } else {
        image::imageops::resize(
            &alpha,
            frame.width(),
            frame.height(),
            image::imageops::FilterType::CatmullRom,
        )
    };
    if !alpha.pixels().any(|pixel| pixel[0] >= 128) {
        return Err("The attached mask does not contain a foreground subject".into());
    }
    if !alpha.pixels().any(|pixel| pixel[0] < 128) {
        return Err("The attached mask must leave some background outside the subject".into());
    }
    let cutout = image::RgbaImage::from_fn(frame.width(), frame.height(), |x, y| {
        let pixel = frame.get_pixel(x, y);
        image::Rgba([pixel[0], pixel[1], pixel[2], alpha.get_pixel(x, y)[0]])
    });
    Ok((
        cutout,
        if has_transparency {
            "alpha"
        } else {
            "luminance"
        },
    ))
}

fn decode_first_frame(
    ffmpeg: &Path,
    input: &Path,
    meta: &VideoMeta,
) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>, String> {
    let (width, height) = preview_dimensions(meta.width, meta.height);
    let decoded = background_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &scale_to_rgb_filter(width, height, &meta.color),
            "-an",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .map_err(|error| format!("Could not decode the first video frame: {error}"))?;
    if !decoded.status.success() {
        let details = String::from_utf8_lossy(&decoded.stderr).trim().to_string();
        return Err(if details.is_empty() {
            "FFmpeg could not decode the first video frame".into()
        } else {
            format!("FFmpeg could not decode the first video frame: {details}")
        });
    }
    ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, decoded.stdout)
        .ok_or_else(|| "FFmpeg returned an incomplete first video frame".into())
}

#[allow(clippy::too_many_arguments)]
pub fn create_video_seed(
    app: &AppHandle,
    control: &JobControl,
    input: &Path,
    source_output: &Path,
    seed_output: &Path,
    model_id: ModelId,
    edge_detail: u8,
    quality: &str,
    imported_mask: Option<&Path>,
) -> Result<SeedOutcome, String> {
    let ffmpeg = bundled_binary(app, "ffmpeg")?;
    let ffprobe = bundled_binary(app, "ffprobe")?;
    let meta = probe(&ffprobe, input)?;
    emit_progress(
        app,
        control,
        "decodingSeed",
        Some(0),
        Some(1),
        None,
        "Decoding the first video frame",
    );
    let decode_started = Instant::now();
    let frame = decode_first_frame(&ffmpeg, input, &meta)?;
    let decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    frame
        .save_with_format(source_output, image::ImageFormat::Png)
        .map_err(|error| format!("Could not save the first video frame: {error}"))?;
    if control.cancelled.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }

    if let Some(mask_path) = imported_mask {
        emit_progress(
            app,
            control,
            "preparingSeed",
            Some(0),
            Some(1),
            None,
            "Preparing the attached first-frame mask",
        );
        let supplied = crate::media_limits::open_image(mask_path)
            .map_err(|error| format!("Could not open the attached PNG mask: {error}"))?;
        let (cutout, mask_kind) = prepare_imported_seed(&frame, supplied)?;
        cutout
            .save_with_format(seed_output, image::ImageFormat::Png)
            .map_err(|error| format!("Could not save the attached first-frame mask: {error}"))?;
        emit_progress(
            app,
            control,
            "preparingSeed",
            Some(1),
            Some(1),
            None,
            "Attached first-frame mask ready to refine",
        );
        return Ok(SeedOutcome {
            provider: "Imported".into(),
            precision: mask_kind.into(),
            width: frame.width(),
            height: frame.height(),
            performance: PerformanceMetrics {
                decode_ms,
                preprocess_ms: 0.0,
                inference_ms: 0.0,
                postprocess_ms: 0.0,
                temporal_and_composite_ms: 0.0,
                encode_ms: 0.0,
                first_inference_ms: None,
            },
        });
    }

    app.state::<crate::cutie::CutieSessionCache>().invalidate();
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
                    "Loading first-frame segmentation model",
                )
            },
            |masker, _| {
                emit_progress(
                    app,
                    control,
                    "preparingSeed",
                    Some(0),
                    Some(1),
                    None,
                    "Creating the editable first-frame mask",
                );
                let cutout = masker.apply(
                    &DynamicImage::ImageRgb8(frame),
                    edge_detail,
                    quality,
                    control,
                )?;
                save_cutout(&cutout, seed_output)?;
                let timing = masker.last_timing();
                let outcome = SeedOutcome {
                    provider: masker.provider().into(),
                    precision: masker.precision().into(),
                    width: cutout.width(),
                    height: cutout.height(),
                    performance: PerformanceMetrics {
                        decode_ms,
                        preprocess_ms: timing.preprocess.as_secs_f64() * 1000.0,
                        inference_ms: timing.inference.as_secs_f64() * 1000.0,
                        postprocess_ms: timing.postprocess.as_secs_f64() * 1000.0,
                        temporal_and_composite_ms: 0.0,
                        encode_ms: 0.0,
                        first_inference_ms: Some(timing.inference.as_secs_f64() * 1000.0),
                    },
                };
                masker.recycle_cutout(cutout);
                emit_progress(
                    app,
                    control,
                    "preparingSeed",
                    Some(1),
                    Some(1),
                    None,
                    "First-frame mask ready to refine",
                );
                Ok(outcome)
            },
        )
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
    tracking_mode: &str,
    seed_mask: Option<&Path>,
    cutie_tier: crate::cutie_models::CutieTier,
) -> Result<VideoOutcome, String> {
    let ffmpeg = bundled_binary(app, "ffmpeg")?;
    let ffprobe = bundled_binary(app, "ffprobe")?;
    let model_path = crate::models::model_path(app, model_id)?;
    if !preview && tracking_mode == "cutie" {
        app.state::<crate::inference::ModelSessionCache>()
            .invalidate_all();
        let seed_mask = seed_mask.ok_or("Primary Subject mode requires a first-frame mask")?;
        let paths = crate::cutie_models::model_paths(app, cutie_tier)?;
        return app.state::<crate::cutie::CutieSessionCache>().with_tracker(
            paths,
            |tracker, reused| {
                emit_progress(
                    app,
                    control,
                    "loadingModel",
                    None,
                    None,
                    None,
                    if reused {
                        "Using loaded Cutie tracker"
                    } else {
                        "Loading Cutie tracker"
                    },
                );
                process_video_with_cutie(
                    Some(app),
                    control,
                    input,
                    output,
                    tracker,
                    &ffmpeg,
                    &ffprobe,
                    quality,
                    screen_color,
                    seed_mask,
                )
            },
        );
    }
    app.state::<crate::cutie::CutieSessionCache>().invalidate();
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
fn process_video_with_cutie(
    app: Option<&AppHandle>,
    control: &JobControl,
    input: &Path,
    output: &Path,
    tracker: &mut crate::cutie::CutieTracker,
    ffmpeg: &Path,
    ffprobe: &Path,
    quality: &str,
    screen_color: &str,
    seed_path: &Path,
) -> Result<VideoOutcome, String> {
    let quality_mode = QualityMode::parse(quality)?;
    let meta = probe(ffprobe, input)?;
    let (width, height, total) = (meta.width, meta.height, meta.frames);
    let seed_image = crate::media_limits::open_image(seed_path)
        .map_err(|error| format!("Could not open first-frame mask: {error}"))?;
    let rgba = seed_image.to_rgba8();
    let has_alpha = rgba.pixels().any(|pixel| pixel[3] < 255);
    let seed = GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y);
        Luma([if has_alpha {
            pixel[3]
        } else {
            ((pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3) as u8
        }])
    });
    if !seed.pixels().any(|pixel| pixel[0] >= 128) {
        return Err("The first-frame mask does not contain a foreground subject".into());
    }
    let video_encoder = if tracker.provider() == "DmlExecutionProvider" {
        VideoEncoder::Software
    } else {
        select_video_encoder(ffmpeg, width, height)
    };
    if let Some(app) = app {
        emit_progress(
            app,
            control,
            "processingFrames",
            Some(0),
            Some(total),
            None,
            format!(
                "Preparing {} Cutie tracking and {} encoding",
                tracker.provider().trim_end_matches("ExecutionProvider"),
                video_encoder.label()
            ),
        );
    }

    let decode_filter = format!(
        "fps={},{}",
        meta.fps_arg,
        scale_to_rgb_filter(width, height, &meta.color)
    );
    let mut decoder = background_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args([
            "-vf",
            &decode_filter,
            "-frames:v",
            &total.to_string(),
            "-an",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map(FfmpegChild::new)
        .map_err(|e| format!("Could not start video decoder: {e}"))?;
    let clip_duration = meta.duration;
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
        meta.fps_arg.clone(),
        "-i".into(),
        "pipe:0".into(),
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-map".into(),
        "0:v:0".into(),
    ];
    if meta.has_audio {
        encode_args.extend(["-map".into(), "1:a:0?".into()]);
    } else {
        encode_args.push("-an".into());
    }
    append_video_encoder_args(&mut encode_args, quality_mode, video_encoder);
    encode_args.extend([
        "-vf".into(),
        rgb_to_yuv_filter(&meta.color),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-fps_mode".into(),
        "cfr".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-metadata:s:v:0".into(),
        "rotate=0".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    append_color_args(&mut encode_args, &meta.color, video_encoder);
    if meta.has_audio {
        encode_args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into(), "-af".into(), format!("atrim=duration={clip_duration:.6},asetpts=PTS-STARTPTS,aresample=async=1000:first_pts=0")]);
    }
    encode_args.push(output.to_string_lossy().into_owned());
    let mut encoder = background_command(ffmpeg)
        .args(&encode_args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map(FfmpegChild::new)
        .map_err(|e| {
            kill(&mut decoder);
            format!("Could not start video encoder: {e}")
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
    let frame_size = crate::media_limits::video_frame_bytes(width, height)?;
    let mut bytes = vec![0_u8; frame_size];
    let mut composited = Vec::with_capacity(frame_size);
    let mut frame_count = 0_u64;
    let mut ewma: Option<f64> = None;
    let mut last_emit = Instant::now();
    let mut performance = PerformanceMetrics {
        decode_ms: 0.0,
        preprocess_ms: 0.0,
        inference_ms: 0.0,
        postprocess_ms: 0.0,
        temporal_and_composite_ms: 0.0,
        encode_ms: 0.0,
        first_inference_ms: None,
    };
    let mut result = Ok(());
    for index in 0..total {
        if control.cancelled.load(Ordering::SeqCst) {
            result = Err("cancelled".into());
            break;
        }
        let started = Instant::now();
        let decode = Instant::now();
        if let Err(error) = reader.read_exact(&mut bytes) {
            if error.kind() != std::io::ErrorKind::UnexpectedEof {
                result = Err(format!("Could not decode video frame: {error}"));
            }
            break;
        }
        performance.decode_ms += decode.elapsed().as_secs_f64() * 1000.0;
        let frame =
            match ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, std::mem::take(&mut bytes)) {
                Some(frame) => frame,
                None => {
                    result = Err("Could not construct video frame".into());
                    break;
                }
            };
        let infer = Instant::now();
        let mask = tracker.step(&frame, (index == 0).then_some(&seed), control);
        let infer_ms = infer.elapsed().as_secs_f64() * 1000.0;
        performance.inference_ms += infer_ms;
        performance.first_inference_ms.get_or_insert(infer_ms);
        let mask = match mask {
            Ok(mask) => mask,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        let composite = Instant::now();
        crate::temporal::composite_rgb_alpha(
            frame.as_raw(),
            mask.as_raw(),
            screen_color,
            &mut composited,
        );
        performance.temporal_and_composite_ms += composite.elapsed().as_secs_f64() * 1000.0;
        let encode = Instant::now();
        if let Err(error) = writer.write_all(&composited) {
            result = Err(format!("Could not encode video frame: {error}"));
            break;
        }
        performance.encode_ms += encode.elapsed().as_secs_f64() * 1000.0;
        bytes = frame.into_raw();
        frame_count += 1;
        let seconds = started.elapsed().as_secs_f64();
        ewma = Some(ewma.map(|old| old * 0.8 + seconds * 0.2).unwrap_or(seconds));
        if let Some(app) = app
            .filter(|_| frame_count == total || last_emit.elapsed() >= Duration::from_millis(200))
        {
            let eta = (frame_count >= 3 && total > frame_count)
                .then(|| (ewma.unwrap_or(0.0) * (total - frame_count) as f64).ceil() as u64);
            emit_progress(
                app,
                control,
                "trackingFrames",
                Some(frame_count),
                Some(total.max(frame_count)),
                eta,
                format!("Tracking primary subject in frame {frame_count} of {total}"),
            );
            last_emit = Instant::now();
        }
    }
    drop(writer);
    if let Err(error) = result {
        kill(&mut decoder);
        kill(&mut encoder);
        let _ = std::fs::remove_file(output);
        return Err(error);
    }
    let decoder_status = decoder.wait().map_err(|e| e.to_string())?;
    let encoder_status = encoder.wait().map_err(|e| e.to_string())?;
    if !decoder_status.success() || !encoder_status.success() {
        let details = [child_error(&mut decoder), child_error(&mut encoder)]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        let _ = std::fs::remove_file(output);
        return Err(if details.is_empty() {
            "FFmpeg could not finish the tracked export".into()
        } else {
            details
        });
    }
    if frame_count == 0 {
        let _ = std::fs::remove_file(output);
        return Err("Cutie did not receive a video frame".into());
    }
    let output_fps = parse_rate(&meta.fps_arg).unwrap_or(30.0);
    validate_export(
        ffprobe,
        output,
        width,
        height,
        clip_duration,
        meta.has_audio,
        output_fps,
    )?;
    let (tracker_width, tracker_height) = tracker.internal_size();
    Ok(VideoOutcome {
        frame_count,
        provider: tracker.provider().into(),
        precision: "FP32".into(),
        pipeline: format!(
            "Cutie primary-subject memory propagation at {tracker_width}x{tracker_height}"
        ),
        performance,
        width,
        height,
        frame_rate: output_fps,
        duration: clip_duration,
        has_audio: meta.has_audio,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn process_video_with_cutie_paths(
    control: &JobControl,
    input: &Path,
    output: &Path,
    model_paths: &crate::cutie::CutieModelPaths,
    ffmpeg: &Path,
    ffprobe: &Path,
    quality: &str,
    screen_color: &str,
    seed_path: &Path,
) -> Result<VideoOutcome, String> {
    let mut tracker = crate::cutie::CutieTracker::load(model_paths, true)?;
    tracker.reset();
    process_video_with_cutie(
        None,
        control,
        input,
        output,
        &mut tracker,
        ffmpeg,
        ffprobe,
        quality,
        screen_color,
        seed_path,
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
fn process_preview_frame(
    app: Option<&AppHandle>,
    control: &JobControl,
    input: &Path,
    output: &Path,
    masker: &mut Masker,
    ffmpeg: &Path,
    edge_detail: u8,
    quality: &str,
    screen_color: &str,
    start_seconds: f64,
    meta: &VideoMeta,
) -> Result<VideoOutcome, String> {
    if control.cancelled.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }
    let fps = parse_rate(&meta.fps_arg).unwrap_or(30.0).max(1.0);
    let last_frame_time = (meta.duration - 1.0 / fps).max(0.0);
    let start = start_seconds.max(0.0).min(last_frame_time);
    let (width, height) = preview_dimensions(meta.width, meta.height);
    if let Some(app) = app {
        emit_progress(
            app,
            control,
            "processingFrames",
            Some(0),
            Some(PREVIEW_FRAME_COUNT),
            None,
            "Processing preview frame",
        );
    }
    let decode_started = Instant::now();
    let decoded = background_command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-ss"])
        .arg(format!("{start:.6}"))
        .arg("-i")
        .arg(input)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &scale_to_rgb_filter(width, height, &meta.color),
            "-an",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .map_err(|error| format!("Could not decode the preview frame: {error}"))?;
    let decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    if !decoded.status.success() {
        let details = String::from_utf8_lossy(&decoded.stderr).trim().to_string();
        return Err(if details.is_empty() {
            "FFmpeg could not decode the preview frame".into()
        } else {
            format!("FFmpeg could not decode the preview frame: {details}")
        });
    }
    let frame = ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, decoded.stdout)
        .ok_or("FFmpeg returned an incomplete preview frame")?;
    let cutout = masker.apply(
        &DynamicImage::ImageRgb8(frame),
        edge_detail,
        quality,
        control,
    )?;
    let timing = masker.last_timing();
    let composite_started = Instant::now();
    let composited_bytes = composite_screen(&cutout, screen_color);
    masker.recycle_cutout(cutout);
    let composited = ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, composited_bytes)
        .ok_or("Could not construct the preview image")?;
    composited
        .save_with_format(output, image::ImageFormat::Png)
        .map_err(|error| format!("Could not save the preview frame: {error}"))?;
    let temporal_and_composite_ms = composite_started.elapsed().as_secs_f64() * 1000.0;
    if let Some(app) = app {
        emit_progress(
            app,
            control,
            "processingFrames",
            Some(PREVIEW_FRAME_COUNT),
            Some(PREVIEW_FRAME_COUNT),
            None,
            "Preview frame ready",
        );
    }
    Ok(VideoOutcome {
        frame_count: PREVIEW_FRAME_COUNT,
        provider: masker.provider().into(),
        precision: masker.precision().into(),
        pipeline: "BiRefNet temporal preview".into(),
        performance: PerformanceMetrics {
            decode_ms,
            preprocess_ms: timing.preprocess.as_secs_f64() * 1000.0,
            inference_ms: timing.inference.as_secs_f64() * 1000.0,
            postprocess_ms: timing.postprocess.as_secs_f64() * 1000.0,
            temporal_and_composite_ms,
            encode_ms: 0.0,
            first_inference_ms: Some(timing.inference.as_secs_f64() * 1000.0),
        },
        width,
        height,
        frame_rate: fps,
        duration: 0.0,
        has_audio: false,
    })
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
    if preview {
        return process_preview_frame(
            app,
            control,
            input,
            output,
            masker,
            ffmpeg,
            edge_detail,
            quality,
            screen_color,
            start_seconds,
            &meta,
        );
    }
    let clip_duration = meta.duration;
    let (width, height, fps_arg, total) =
        (meta.width, meta.height, meta.fps_arg.clone(), meta.frames);
    if control.cancelled.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }
    // On some Windows/NVIDIA driver combinations, initializing NVENC while a
    // DirectML session is active removes the D3D device used by ONNX Runtime.
    // Segmentation is the dominant cost, so preserve GPU inference and use
    // NVENC to occupy an otherwise idle GPU only after inference fell back to
    // CPU during session creation.
    let video_encoder = if masker.provider() == "DmlExecutionProvider" {
        VideoEncoder::Software
    } else {
        select_video_encoder(ffmpeg, width, height)
    };
    if let Some(app) = app {
        emit_progress(
            app,
            control,
            "processingFrames",
            Some(0),
            Some(total),
            None,
            format!(
                "Preparing {} {} inference and {} encoding",
                masker.provider().trim_end_matches("ExecutionProvider"),
                masker.precision(),
                video_encoder.label()
            ),
        );
    }

    let mut decode_args = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    decode_args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    let decode_filter = format!(
        "fps={},{}",
        meta.fps_arg,
        scale_to_rgb_filter(width, height, &meta.color)
    );
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
    let mut decoder = background_command(ffmpeg)
        .args(&decode_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map(FfmpegChild::new)
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
    encode_args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    encode_args.extend(["-map".into(), "0:v:0".into()]);
    if meta.has_audio {
        encode_args.extend(["-map".into(), "1:a:0?".into()]);
    } else {
        encode_args.push("-an".into());
    }
    append_video_encoder_args(&mut encode_args, quality_mode, video_encoder);
    encode_args.extend(["-vf".into(), rgb_to_yuv_filter(&meta.color)]);
    encode_args.extend([
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-fps_mode".into(),
        "cfr".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-metadata:s:v:0".into(),
        "rotate=0".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    append_color_args(&mut encode_args, &meta.color, video_encoder);
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
    let mut encoder = background_command(ffmpeg)
        .args(&encode_args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map(FfmpegChild::new)
        .map_err(|error| {
            kill(&mut decoder);
            format!("Could not start video encoder: {error}")
        })?;

    let reader = match decoder.stdout.take() {
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
    let frame_size = crate::media_limits::video_frame_bytes(width, height)?;
    let (prepared_tx, prepared_rx) =
        mpsc::sync_channel::<Result<(PreparedFrame, Duration), String>>(2);
    let (recycle_tx, recycle_rx) = mpsc::sync_channel::<(Vec<u8>, Vec<f32>)>(2);
    let quality_for_pipeline = quality.to_string();
    let pipeline_model_id = masker.model_id();
    let cancelled = control.cancelled.clone();
    let pipeline = thread::spawn(move || {
        let mut reader = reader;
        let mut buffers = VecDeque::from([
            (vec![0_u8; frame_size], Vec::new()),
            (vec![0_u8; frame_size], Vec::new()),
        ]);
        for _ in 0..total {
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            let (mut bytes, input) = match buffers.pop_front() {
                Some(buffers) => buffers,
                None => match recycle_rx.recv() {
                    Ok(buffers) => buffers,
                    Err(_) => break,
                },
            };
            let decode_started = Instant::now();
            if let Err(error) = reader.read_exact(&mut bytes) {
                if error.kind() != std::io::ErrorKind::UnexpectedEof {
                    let _ = prepared_tx.send(Err(format!("Could not decode video frame: {error}")));
                }
                break;
            }
            let decode = decode_started.elapsed();
            let Some(image) = ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, bytes) else {
                let _ = prepared_tx.send(Err("Could not construct video frame".into()));
                break;
            };
            let prepared =
                prepare_video_frame(image, pipeline_model_id, &quality_for_pipeline, input);
            if prepared_tx
                .send(prepared.map(|frame| (frame, decode)))
                .is_err()
            {
                break;
            }
            while let Ok(recycled) = recycle_rx.try_recv() {
                buffers.push_back(recycled);
            }
        }
        Ok::<(), String>(())
    });
    let mut composited = Vec::with_capacity(frame_size);
    let mut frame_count = 0_u64;
    let mut inferred_count = 0_u64;
    let mut ewma: Option<f64> = None;
    let mut stabilizer = TemporalMaskStabilizer::default();
    let mut last_progress_emit = Instant::now();
    let mut performance = PerformanceMetrics {
        decode_ms: 0.0,
        preprocess_ms: 0.0,
        inference_ms: 0.0,
        postprocess_ms: 0.0,
        temporal_and_composite_ms: 0.0,
        encode_ms: 0.0,
        first_inference_ms: None,
    };
    let mut result = loop {
        if control.cancelled.load(Ordering::SeqCst) {
            break Err("cancelled".into());
        }
        let (prepared, decode) = match prepared_rx.recv() {
            Ok(Ok(frame)) => frame,
            Ok(Err(error)) => break Err(error),
            Err(_) => break Ok(()),
        };
        performance.decode_ms += decode.as_secs_f64() * 1000.0;
        let frame_started = Instant::now();
        let cutout = match masker.apply_prepared(&prepared, edge_detail, control) {
            Ok(cutout) => cutout,
            Err(error) => break Err(error),
        };
        let timing = masker.last_timing();
        performance.preprocess_ms += timing.preprocess.as_secs_f64() * 1000.0;
        performance.inference_ms += timing.inference.as_secs_f64() * 1000.0;
        performance.postprocess_ms += timing.postprocess.as_secs_f64() * 1000.0;
        performance
            .first_inference_ms
            .get_or_insert(timing.inference.as_secs_f64() * 1000.0);
        let temporal_started = Instant::now();
        let temporal_result = stabilizer.push_frame(
            prepared.source_bytes(),
            &cutout,
            screen_color,
            &mut composited,
        );
        performance.temporal_and_composite_ms += temporal_started.elapsed().as_secs_f64() * 1000.0;
        masker.recycle_cutout(cutout);
        let _ = recycle_tx.send(prepared.into_recycling_parts());
        let output_ready = match temporal_result {
            Ok(output_ready) => output_ready,
            Err(error) => break Err(error),
        };
        if output_ready {
            let encode_started = Instant::now();
            if let Err(error) = writer.write_all(&composited) {
                break Err(format!("Could not encode video frame: {error}"));
            }
            performance.encode_ms += encode_started.elapsed().as_secs_f64() * 1000.0;
            frame_count += 1;
        }
        inferred_count += 1;
        let seconds = frame_started.elapsed().as_secs_f64();
        ewma = Some(ewma.map(|old| old * 0.8 + seconds * 0.2).unwrap_or(seconds));
        let eta = (inferred_count >= 3 && total > inferred_count)
            .then(|| (ewma.unwrap_or(0.0) * (total - inferred_count) as f64).ceil() as u64);
        if let Some(app) = app.filter(|_| {
            inferred_count == total || last_progress_emit.elapsed() >= Duration::from_millis(200)
        }) {
            emit_progress(
                app,
                control,
                "processingFrames",
                Some(inferred_count),
                Some(total.max(inferred_count)),
                eta,
                format!(
                    "Processing frame {inferred_count} of {total} with {} and {}",
                    masker.provider().trim_end_matches("ExecutionProvider"),
                    video_encoder.label()
                ),
            );
            last_progress_emit = Instant::now();
        }
    };
    if result.is_ok() && control.cancelled.load(Ordering::SeqCst) {
        result = Err("cancelled".into());
    }
    if result.is_ok() {
        let temporal_started = Instant::now();
        let final_output = stabilizer.finish(screen_color, &mut composited);
        performance.temporal_and_composite_ms += temporal_started.elapsed().as_secs_f64() * 1000.0;
        match final_output {
            Ok(true) => {
                let encode_started = Instant::now();
                if let Err(error) = writer.write_all(&composited) {
                    result = Err(format!("Could not encode final video frame: {error}"));
                } else {
                    performance.encode_ms += encode_started.elapsed().as_secs_f64() * 1000.0;
                    frame_count += 1;
                }
            }
            Ok(false) => {}
            Err(error) => result = Err(error),
        }
    }
    if result.is_ok() && frame_count != inferred_count {
        result = Err(format!(
            "Temporal pipeline encoded {frame_count} of {inferred_count} inferred frames"
        ));
    }
    drop(recycle_tx);
    drop(writer);
    if let Err(error) = result {
        kill(&mut decoder);
        kill(&mut encoder);
        drop(prepared_rx);
        let _ = pipeline.join();
        let _ = std::fs::remove_file(output);
        return Err(error);
    }
    let pipeline_result = pipeline
        .join()
        .map_err(|_| "Video preprocessing pipeline panicked".to_string())?;
    pipeline_result?;
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
        precision: masker.precision().into(),
        pipeline: "BiRefNet + three-frame motion-gated temporal matte".into(),
        performance,
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
    fn stderr_drain_keeps_only_the_bounded_tail() {
        let mut bytes = vec![b'x'; STDERR_TAIL_BYTES * 4];
        bytes.extend_from_slice(b"last diagnostic");
        let tail = drain_stderr(std::io::Cursor::new(&bytes));
        assert_eq!(tail.len(), STDERR_TAIL_BYTES);
        assert_eq!(tail, bytes[bytes.len() - STDERR_TAIL_BYTES..]);
    }

    #[cfg(windows)]
    #[test]
    fn noisy_child_finishes_with_bounded_diagnostics() {
        let child = background_command(Path::new("powershell.exe"))
            .args([
                "-NoProfile",
                "-Command",
                "[Console]::Error.Write(('x' * 262144) + 'END')",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut child = FfmpegChild::new(child);
        assert!(child.wait().unwrap().success());
        assert_eq!(child.stderr_tail.len(), STDERR_TAIL_BYTES);
        assert!(child.stderr_tail.ends_with(b"END"));
        assert!(child.stderr_worker.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn early_return_reaps_child_and_joins_stderr() {
        let child = background_command(Path::new("powershell.exe"))
            .args([
                "-NoProfile",
                "-Command",
                "[Console]::Error.Write('started'); Start-Sleep -Seconds 60",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        drop(FfmpegChild::new(child));
        let status = background_command(Path::new("powershell.exe"))
            .args([
                "-NoProfile",
                "-Command",
                &format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 1 }}"),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "child survived ownership cleanup");
    }

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
            color_range: None,
            color_space: None,
            color_transfer: None,
            color_primaries: None,
            tags: None,
            side_data_list: Some(vec![ProbeSideData {
                rotation: Some(-90.0),
            }]),
            disposition: None,
        };
        assert_eq!(normalized_rotation(&stream), 270);
    }

    #[test]
    fn preview_output_is_a_single_frame_image() {
        assert_eq!(PREVIEW_FRAME_COUNT, 1);
    }

    #[test]
    fn quality_modes_have_distinct_video_profiles() {
        assert_eq!(encoding_profile(QualityMode::Fast), ("23", "veryfast"));
        assert_eq!(encoding_profile(QualityMode::Balanced), ("19", "fast"));
        assert_eq!(encoding_profile(QualityMode::Maximum), ("16", "slow"));
    }

    #[test]
    fn hardware_encoder_uses_nvenc_and_software_encoder_uses_x264() {
        let mut hardware = Vec::new();
        append_video_encoder_args(&mut hardware, QualityMode::Balanced, VideoEncoder::Nvidia);
        assert!(hardware.iter().any(|value| value == "h264_nvenc"));
        assert!(hardware.iter().any(|value| value == "p4"));

        let mut software = Vec::new();
        append_video_encoder_args(&mut software, QualityMode::Balanced, VideoEncoder::Software);
        assert!(software.iter().any(|value| value == "libx264"));
        assert!(software.iter().any(|value| value == "fast"));
    }

    #[test]
    fn source_color_tags_are_preserved_in_conversion_and_output_args() {
        let stream = ProbeStream {
            codec_type: Some("video".into()),
            width: Some(1920),
            height: Some(1080),
            avg_frame_rate: None,
            r_frame_rate: None,
            duration: None,
            nb_frames: None,
            sample_aspect_ratio: None,
            color_range: Some("pc".into()),
            color_space: Some("bt709".into()),
            color_transfer: Some("bt709".into()),
            color_primaries: Some("bt709".into()),
            tags: None,
            side_data_list: None,
            disposition: None,
        };
        let color = color_spec(&stream, 1920, 1080);
        assert_eq!(color.range, "pc");
        assert!(scale_to_rgb_filter(1920, 1080, &color).contains("in_color_matrix=bt709"));
        assert!(rgb_to_yuv_filter(&color).contains("out_range=pc"));

        let mut args = Vec::new();
        append_color_args(&mut args, &color, VideoEncoder::Software);
        assert!(args.windows(2).any(|pair| pair == ["-colorspace", "bt709"]));
        assert!(args.windows(2).any(|pair| pair == ["-color_range", "pc"]));
        assert!(args
            .iter()
            .any(|argument| argument
                == "colorprim=bt709:transfer=bt709:colormatrix=bt709:fullrange=on"));
    }

    #[test]
    fn untagged_sdr_uses_conventional_sd_and_hd_matrices() {
        let stream = ProbeStream {
            codec_type: Some("video".into()),
            width: None,
            height: None,
            avg_frame_rate: None,
            r_frame_rate: None,
            duration: None,
            nb_frames: None,
            sample_aspect_ratio: None,
            color_range: None,
            color_space: None,
            color_transfer: None,
            color_primaries: None,
            tags: None,
            side_data_list: None,
            disposition: None,
        };
        assert_eq!(color_spec(&stream, 1920, 1080).space, "bt709");
        assert_eq!(color_spec(&stream, 720, 480).filter_matrix, "bt601");
    }

    #[test]
    fn transparent_imported_mask_uses_alpha_and_frame_rgb() {
        let frame = ImageBuffer::from_pixel(16, 9, Rgb([12, 34, 56]));
        let mask = image::RgbaImage::from_fn(32, 18, |x, _| {
            image::Rgba([200, 10, 10, if x < 16 { 255 } else { 0 }])
        });
        let (cutout, kind) = prepare_imported_seed(&frame, DynamicImage::ImageRgba8(mask)).unwrap();

        assert_eq!(kind, "alpha");
        assert_eq!(cutout.dimensions(), frame.dimensions());
        assert_eq!(&cutout.get_pixel(0, 0).0[..3], &[12, 34, 56]);
        assert_eq!(cutout.get_pixel(0, 0)[3], 255);
        assert_eq!(cutout.get_pixel(15, 0)[3], 0);
    }

    #[test]
    fn opaque_imported_mask_uses_black_and_white_luminance() {
        let frame = ImageBuffer::from_pixel(16, 9, Rgb([80, 90, 100]));
        let mask = image::RgbImage::from_fn(16, 9, |x, _| {
            if x < 8 {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            }
        });
        let (cutout, kind) = prepare_imported_seed(&frame, DynamicImage::ImageRgb8(mask)).unwrap();

        assert_eq!(kind, "luminance");
        assert_eq!(cutout.get_pixel(0, 0)[3], 255);
        assert_eq!(cutout.get_pixel(15, 0)[3], 0);
    }

    #[test]
    fn imported_mask_rejects_wrong_aspect_ratio() {
        let frame = ImageBuffer::from_pixel(16, 9, Rgb([0, 0, 0]));
        let mask = DynamicImage::ImageRgba8(image::RgbaImage::new(16, 16));

        assert!(prepare_imported_seed(&frame, mask)
            .unwrap_err()
            .contains("same aspect ratio"));
    }

    #[test]
    fn imported_mask_requires_foreground_and_background() {
        let frame = ImageBuffer::from_pixel(16, 9, Rgb([0, 0, 0]));
        let all_subject =
            DynamicImage::ImageRgb8(ImageBuffer::from_pixel(16, 9, Rgb([255, 255, 255])));
        let no_subject = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(16, 9, Rgb([0, 0, 0])));

        assert!(prepare_imported_seed(&frame, all_subject)
            .unwrap_err()
            .contains("leave some background"));
        assert!(prepare_imported_seed(&frame, no_subject)
            .unwrap_err()
            .contains("does not contain a foreground"));
    }
}
