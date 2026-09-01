use roto_now_lib::{
    cutie::CutieModelPaths, jobs::JobControl, video::process_video_with_cutie_paths,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc,
    },
};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 8 {
        return Err(
            "usage: cutie_video_smoke <model-dir> <ffmpeg.exe> <ffprobe.exe> <input-video> <seed-cutout.png> <output.mp4> <green|blue>"
                .into(),
        );
    }
    let root = PathBuf::from(&args[1]);
    let (width, height) = if std::env::var("ROTO_NOW_CUTIE_TIER").as_deref() == Ok("high") {
        (960, 544)
    } else {
        (640, 368)
    };
    let size = format!("{width}x{height}");
    let paths = CutieModelPaths {
        encode_key: root.join(format!("cutie-encode-key-{size}.onnx")),
        encode_value: root.join(format!("cutie-encode-value-{size}.onnx")),
        memory_readout: root.join(format!(
            "cutie-memory-readout-floatmask-valid-{size}-m6-topk30-opencv.onnx"
        )),
        decode: root.join(format!("cutie-decode-{size}.onnx")),
        width,
        height,
    };
    let control = JobControl {
        id: "cutie-video-smoke".into(),
        cancelled: Arc::new(AtomicBool::new(false)),
        progress_high_water: Arc::new(AtomicU64::new(0)),
    };
    let result = process_video_with_cutie_paths(
        &control,
        PathBuf::from(&args[4]).as_path(),
        PathBuf::from(&args[6]).as_path(),
        &paths,
        PathBuf::from(&args[2]).as_path(),
        PathBuf::from(&args[3]).as_path(),
        "Balanced",
        &args[7],
        PathBuf::from(&args[5]).as_path(),
    )?;
    println!(
        "frames={} size={}x{} fps={:.3} duration={:.3} audio={} provider={} inference_ms={:.1}",
        result.frame_count,
        result.width,
        result.height,
        result.frame_rate,
        result.duration,
        result.has_audio,
        result.provider,
        result.performance.inference_ms,
    );
    Ok(())
}
