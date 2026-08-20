use roto_now_lib::{jobs::JobControl, models::ModelId, video::process_video_with_paths};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc,
    },
};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 7 {
        return Err("usage: video_smoke <model.onnx> <ffmpeg.exe> <ffprobe.exe> <input> <output.png|output.mp4> <preview|fast|full|maximum|cancel>".into());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = JobControl {
        id: "video-smoke".into(),
        cancelled: Arc::clone(&cancelled),
        progress_high_water: Arc::new(AtomicU64::new(0)),
    };
    let preview = args[6] == "preview";
    let model_id = match std::env::var("ROTO_NOW_MODEL").as_deref() {
        Ok("anime") => ModelId::Anime,
        Ok("general") => ModelId::General,
        _ => ModelId::GeneralLite,
    };
    let quality = match args[6].as_str() {
        "fast" => "Fast",
        "maximum" => "Maximum",
        _ => "Balanced",
    };
    if args[6] == "cancel" {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(8));
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }
    let result = process_video_with_paths(
        None,
        &control,
        PathBuf::from(&args[4]).as_path(),
        PathBuf::from(&args[5]).as_path(),
        model_id,
        PathBuf::from(&args[1]).as_path(),
        PathBuf::from(&args[2]).as_path(),
        PathBuf::from(&args[3]).as_path(),
        72,
        quality,
        "green",
        preview,
        0.0,
    );
    if args[6] == "cancel" {
        match result {
            Err(error) if error == "cancelled" => {
                println!("cancelled cleanly");
                return Ok(());
            }
            other => {
                return Err(format!(
                    "expected cancellation, got {}",
                    other.err().unwrap_or_else(|| "success".into())
                ))
            }
        }
    }
    let result = result?;
    println!(
        "frames={} size={}x{} fps={:.3} duration={:.3} audio={} provider={} precision={} inference_ms={:.1} preprocess_ms={:.1}",
        result.frame_count,
        result.width,
        result.height,
        result.frame_rate,
        result.duration,
        result.has_audio,
        result.provider,
        result.precision,
        result.performance.inference_ms,
        result.performance.preprocess_ms,
    );
    Ok(())
}
