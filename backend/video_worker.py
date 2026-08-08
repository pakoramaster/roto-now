from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

import imageio_ffmpeg
import numpy as np
import onnxruntime as ort
from PIL import Image
from rembg import new_session, remove

from worker import TOONOUT_URL, download_file, providers_for_onnx, refine_alpha


def emit_error(message: str) -> None:
    print(json.dumps({"ok": False, "error": message}), flush=True)


class GeneralMasker:
    def __init__(self, quality: str):
        if quality == "maximum":
            self.model_name = "birefnet-general"
            providers = ["CPUExecutionProvider"]
        else:
            self.model_name = "birefnet-general-lite"
            providers = providers_for_onnx(ort)
        self.session = new_session(self.model_name, providers=providers)
        self.provider = self.session.inner_session.get_providers()[0]

    def apply(self, source: Image.Image, edge_detail: int) -> Image.Image:
        try:
            result = remove(source.convert("RGBA"), session=self.session).convert("RGBA")
        except Exception:
            if self.provider != "DmlExecutionProvider":
                raise
            # DirectML can exhaust VRAM on long or high-resolution clips. Retry the
            # current frame on CPU and keep that session for the rest of the export.
            self.session = new_session(self.model_name, providers=["CPUExecutionProvider"])
            self.provider = "CPUExecutionProvider"
            result = remove(source.convert("RGBA"), session=self.session).convert("RGBA")
        pixels = np.asarray(result).copy()
        pixels[:, :, 3] = refine_alpha(pixels[:, :, 3], edge_detail)
        return Image.fromarray(pixels, "RGBA")


class AnimeMasker:
    def __init__(self, models_dir: Path):
        model_path = models_dir / "toonout" / "birefnet-toonout-fp16.onnx"
        if not model_path.exists():
            print("Downloading ToonOut model...", flush=True)
            download_file(TOONOUT_URL, model_path)
        options = ort.SessionOptions()
        options.enable_mem_pattern = False
        options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        self.model_path = model_path
        self.session = ort.InferenceSession(
            str(model_path), sess_options=options, providers=providers_for_onnx(ort)
        )
        self.provider = self.session.get_providers()[0]

    def apply(self, source: Image.Image, edge_detail: int) -> Image.Image:
        original_size = source.size
        resized = source.convert("RGB").resize((1024, 1024), Image.Resampling.LANCZOS)
        image = np.asarray(resized).astype(np.float32) / 255.0
        mean = np.array([0.485, 0.456, 0.406], dtype=np.float32)
        std = np.array([0.229, 0.224, 0.225], dtype=np.float32)
        tensor = ((image - mean) / std).transpose(2, 0, 1)[None, ...]
        try:
            output = self.session.run(None, {self.session.get_inputs()[0].name: tensor})[0]
        except Exception:
            if self.provider != "DmlExecutionProvider":
                raise
            self.session = ort.InferenceSession(
                str(self.model_path), providers=["CPUExecutionProvider"]
            )
            self.provider = "CPUExecutionProvider"
            output = self.session.run(None, {self.session.get_inputs()[0].name: tensor})[0]
        mask = np.squeeze(output)
        if mask.min() < 0.0 or mask.max() > 1.0:
            mask = 1.0 / (1.0 + np.exp(-mask))
        alpha = Image.fromarray(np.clip(mask * 255.0, 0, 255).astype(np.uint8), "L")
        alpha = alpha.resize(original_size, Image.Resampling.LANCZOS)
        result = source.convert("RGBA")
        result.putalpha(Image.fromarray(refine_alpha(np.asarray(alpha), edge_detail), "L"))
        return result


def composite_screen(cutout: Image.Image, screen_color: str) -> Image.Image:
    rgb = (0, 177, 64) if screen_color == "green" else (0, 71, 187)
    background = Image.new("RGBA", cutout.size, (*rgb, 255))
    return Image.alpha_composite(background, cutout).convert("RGB")


def mux_original_audio(ffmpeg: str, silent_video: Path, source: Path, output: Path) -> None:
    command = [
        ffmpeg,
        "-y",
        "-loglevel",
        "error",
        "-i",
        str(silent_video),
        "-i",
        str(source),
        "-map",
        "0:v:0",
        "-map",
        "1:a?",
        "-c:v",
        "copy",
        "-c:a",
        "aac",
        "-shortest",
        str(output),
    ]
    completed = subprocess.run(command, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or "FFmpeg could not mux the output")


def main() -> int:
    parser = argparse.ArgumentParser(description="Roto Now video worker")
    parser.add_argument("--input", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--model", choices=["auto", "general", "anime"], default="auto")
    parser.add_argument("--quality", choices=["fast", "balanced", "maximum"], default="balanced")
    parser.add_argument("--edge-detail", type=int, default=72)
    parser.add_argument("--screen-color", choices=["green", "blue"], default="green")
    parser.add_argument("--models-dir", required=True)
    args = parser.parse_args()

    started = time.perf_counter()
    source = Path(args.input).resolve()
    output = Path(args.output).resolve()
    models_dir = Path(args.models_dir).resolve()
    if not source.is_file():
        emit_error(f"Input file does not exist: {source}")
        return 2
    if output.suffix.lower() != ".mp4":
        emit_error("Video output must use the .mp4 extension")
        return 2

    output.parent.mkdir(parents=True, exist_ok=True)
    models_dir.mkdir(parents=True, exist_ok=True)
    os.environ["U2NET_HOME"] = str(models_dir / "rembg")
    selected_model = "general" if args.model == "auto" else args.model

    reader = None
    writer = None
    try:
        masker = AnimeMasker(models_dir) if selected_model == "anime" else GeneralMasker(args.quality)
        reader = imageio_ffmpeg.read_frames(str(source), pix_fmt="rgb24")
        metadata = next(reader)
        width, height = metadata["size"]
        fps = float(metadata.get("fps") or 30.0)
        frame_count = 0
        ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
        with tempfile.TemporaryDirectory(prefix="roto-now-", dir=str(output.parent)) as temp_dir:
            silent_video = Path(temp_dir) / "video.mp4"
            writer = imageio_ffmpeg.write_frames(
                str(silent_video),
                (width, height),
                fps=fps,
                codec="libx264",
                pix_fmt_in="rgb24",
                output_params=["-pix_fmt", "yuv420p", "-crf", "18", "-preset", "medium"],
                macro_block_size=2,
            )
            writer.send(None)
            for frame_bytes in reader:
                frame = Image.frombytes("RGB", (width, height), frame_bytes)
                writer.send(composite_screen(masker.apply(frame, args.edge_detail), args.screen_color).tobytes())
                frame_count += 1
            writer.close()
            writer = None
            reader.close()
            reader = None
            mux_original_audio(ffmpeg, silent_video, source, output)

        print(
            json.dumps(
                {
                    "ok": True,
                    "outputPath": str(output),
                    "model": selected_model,
                    "provider": masker.provider,
                    "durationMs": round((time.perf_counter() - started) * 1000),
                    "frameCount": frame_count,
                }
            ),
            flush=True,
        )
        return 0
    except Exception as error:
        emit_error(f"Video processing failed: {error}")
        return 1
    finally:
        if writer is not None:
            try:
                writer.close()
            except Exception:
                pass
        if reader is not None:
            try:
                reader.close()
            except Exception:
                pass


if __name__ == "__main__":
    raise SystemExit(main())
