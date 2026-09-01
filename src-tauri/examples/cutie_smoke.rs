use image::GenericImageView;
use roto_now_lib::{
    cutie::{CutieModelPaths, CutieTracker},
    jobs::JobControl,
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
    if args.len() != 6 {
        return Err(
            "usage: cutie_smoke <model-dir> <first-frame> <seed-cutout> <next-frame> <output-mask>"
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
        id: "cutie-smoke".into(),
        cancelled: Arc::new(AtomicBool::new(false)),
        progress_high_water: Arc::new(AtomicU64::new(0)),
    };
    let first = image::open(&args[2])
        .map_err(|error| error.to_string())?
        .to_rgb8();
    let seed_image = image::open(&args[3]).map_err(|error| error.to_string())?;
    if seed_image.dimensions() != first.dimensions() {
        return Err("seed dimensions must match the first frame".into());
    }
    let seed = seed_image.to_rgba8();
    let alpha = image::GrayImage::from_fn(seed.width(), seed.height(), |x, y| {
        image::Luma([seed.get_pixel(x, y)[3]])
    });
    let next = image::open(&args[4])
        .map_err(|error| error.to_string())?
        .to_rgb8();
    let mut tracker = CutieTracker::load(&paths, true)?;
    tracker.step(&first, Some(&alpha), &control)?;
    let output = tracker.step(&next, None, &control)?;
    output
        .save_with_format(&args[5], image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    println!(
        "provider={} size={}x{}",
        tracker.provider(),
        output.width(),
        output.height()
    );
    Ok(())
}
