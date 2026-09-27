"""Create Roto Now's General FP16 model.

The conversion keeps float32 inputs and outputs so native preprocessing and
postprocessing stay unchanged. The generated model is used by both DirectML
and the CPU fallback after compatibility has been validated locally.
"""

from pathlib import Path

import onnx
from onnxconverter_common import float16


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "src-tauri" / "models" / "birefnet-general-lite.onnx"
DESTINATION = ROOT / "src-tauri" / "models" / "birefnet-general-lite-fp16.onnx"


def main() -> None:
    if not SOURCE.is_file():
        raise SystemExit(f"Missing source model: {SOURCE}")

    model = onnx.load(SOURCE)
    converted = float16.convert_float_to_float16(
        model,
        keep_io_types=True,
        disable_shape_infer=False,
    )
    onnx.checker.check_model(converted)
    onnx.save(converted, DESTINATION)
    print(f"Wrote {DESTINATION} ({DESTINATION.stat().st_size:,} bytes)")


if __name__ == "__main__":
    main()
