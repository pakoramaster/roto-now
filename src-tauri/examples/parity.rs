use roto_now_lib::{
    inference::{save_cutout, Masker},
    jobs::JobControl,
    models::ModelId,
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
    if args.len() != 5 {
        return Err(
            "usage: parity <generalLite|general|anime> <model.onnx> <input> <output.png>".into(),
        );
    }
    let model_id = match args[1].as_str() {
        "generalLite" => ModelId::GeneralLite,
        "general" => ModelId::General,
        "anime" => ModelId::Anime,
        _ => return Err("unknown model id".into()),
    };
    let source = image::open(&args[3]).map_err(|error| error.to_string())?;
    let control = JobControl {
        id: "parity".into(),
        cancelled: Arc::new(AtomicBool::new(false)),
        progress_high_water: Arc::new(AtomicU64::new(0)),
    };
    let mut masker = Masker::load_from_path(PathBuf::from(&args[2]), model_id, false)?;
    let output = masker.apply(&source, 72, "Balanced", &control)?;
    save_cutout(&output, PathBuf::from(&args[4]).as_path())?;
    println!("provider={}", masker.provider());
    Ok(())
}
