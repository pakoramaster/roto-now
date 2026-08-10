from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time
from pathlib import Path
from urllib.request import Request, urlopen


TOONOUT_URL = (
    "https://huggingface.co/sprited/birefnet-toonout-onnx/resolve/main/"
    "birefnet-toonout-fp16.onnx?download=true"
)


def emit_error(message: str) -> None:
    print(json.dumps({"ok": False, "error": message}), flush=True)


def download_file(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".part")
    request = Request(url, headers={"User-Agent": "RotoNow/0.1"})
    with urlopen(request, timeout=120) as response, temporary.open("wb") as handle:
        while chunk := response.read(1024 * 1024):
            handle.write(chunk)
    temporary.replace(destination)


def providers_for_onnx(ort_module) -> list[str]:
    available = ort_module.get_available_providers()
    providers: list[str] = []
    if "DmlExecutionProvider" in available:
        providers.append("DmlExecutionProvider")
    providers.append("CPUExecutionProvider")
    return providers


def refine_alpha(alpha, edge_detail: int):
    import numpy as np

    normalized = np.clip(alpha.astype(np.float32) / 255.0, 1e-4, 1.0 - 1e-4)
    strength = 0.72 + (max(0, min(100, edge_detail)) / 100.0) * 0.72
    logits = np.log(normalized / (1.0 - normalized))
    refined = np.clip((1.0 / (1.0 + np.exp(-logits * strength))) * 255.0, 0, 255)
    refined[alpha <= 2] = 0
    refined[alpha >= 253] = 255
    return refined.astype(np.uint8)


def process_general(input_path: Path, output_path: Path, edge_detail: int, quality: str):
    from PIL import Image
    import numpy as np
    import onnxruntime as ort
    from rembg import new_session, remove

    if quality == "maximum":
        model_name = "birefnet-general"
        providers = ["CPUExecutionProvider"]
    else:
        model_name = "birefnet-general-lite"
        providers = providers_for_onnx(ort)
    session = new_session(model_name, providers=providers)
    source = Image.open(input_path).convert("RGBA")
    result = remove(source, session=session).convert("RGBA")
    pixels = np.asarray(result).copy()
    pixels[:, :, 3] = refine_alpha(pixels[:, :, 3], edge_detail)
    Image.fromarray(pixels, "RGBA").save(output_path, "PNG", optimize=True)
    return session.inner_session.get_providers()[0]


def process_anime(input_path: Path, output_path: Path, edge_detail: int, models_dir: Path):
    from PIL import Image
    import numpy as np
    import onnxruntime as ort

    model_path = models_dir / "toonout" / "birefnet-toonout-fp16.onnx"
    if not model_path.exists():
        print("Downloading ToonOut model…", file=sys.stderr, flush=True)
        download_file(TOONOUT_URL, model_path)

    options = ort.SessionOptions()
    options.enable_mem_pattern = False
    options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    providers = providers_for_onnx(ort)
    session = ort.InferenceSession(str(model_path), sess_options=options, providers=providers)

    source = Image.open(input_path).convert("RGB")
    original_size = source.size
    resized = source.resize((1024, 1024), Image.Resampling.LANCZOS)
    image = np.asarray(resized).astype(np.float32) / 255.0
    mean = np.array([0.485, 0.456, 0.406], dtype=np.float32)
    std = np.array([0.229, 0.224, 0.225], dtype=np.float32)
    image = ((image - mean) / std).transpose(2, 0, 1)[None, ...]

    input_name = session.get_inputs()[0].name
    output = session.run(None, {input_name: image})[0]
    mask = np.squeeze(output)
    if mask.min() < 0.0 or mask.max() > 1.0:
        mask = 1.0 / (1.0 + np.exp(-mask))
    mask_image = Image.fromarray(np.clip(mask * 255.0, 0, 255).astype(np.uint8), "L")
    mask_image = mask_image.resize(original_size, Image.Resampling.LANCZOS)

    rgba = Image.open(input_path).convert("RGBA")
    rgba.putalpha(Image.fromarray(refine_alpha(np.asarray(mask_image), edge_detail), "L"))
    rgba.save(output_path, "PNG", optimize=True)
    return session.get_providers()[0]


def main() -> int:
    parser = argparse.ArgumentParser(description="Roto Now local inference worker")
    parser.add_argument("--input", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--model", choices=["general", "anime"], default="general")
    parser.add_argument("--quality", choices=["fast", "balanced", "maximum"], default="balanced")
    parser.add_argument("--edge-detail", type=int, default=72)
    parser.add_argument("--models-dir", required=True)
    args = parser.parse_args()

    started = time.perf_counter()
    input_path = Path(args.input).resolve()
    output_path = Path(args.output).resolve()
    models_dir = Path(args.models_dir).resolve()

    if not input_path.is_file():
        emit_error(f"Input file does not exist: {input_path}")
        return 2
    if output_path.suffix.lower() != ".png":
        emit_error("Image output must use the .png extension")
        return 2

    output_path.parent.mkdir(parents=True, exist_ok=True)
    models_dir.mkdir(parents=True, exist_ok=True)
    os.environ["U2NET_HOME"] = str(models_dir / "rembg")

    selected_model = args.model
    try:
        if selected_model == "anime":
            provider = process_anime(input_path, output_path, args.edge_detail, models_dir)
        else:
            provider = process_general(input_path, output_path, args.edge_detail, args.quality)
    except Exception as error:
        emit_error(f"Inference failed: {error}")
        return 1

    print(
        json.dumps(
            {
                "ok": True,
                "outputPath": str(output_path),
                "model": selected_model,
                "provider": provider,
                "durationMs": round((time.perf_counter() - started) * 1000),
            }
        ),
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
