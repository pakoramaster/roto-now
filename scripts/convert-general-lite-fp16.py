"""Create Roto Now's GPU-only FP16 General Lite model.

The conversion keeps float32 inputs and outputs so native preprocessing and
postprocessing stay unchanged. CPU inference deliberately continues to use
the bundled FP32 model because ONNX Runtime's CPU provider does not implement
all float16 operators used by BiRefNet.
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
