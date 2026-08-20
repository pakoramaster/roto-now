# Third-party notices

Roto Now packages the following third-party components in its Windows installer.

## FFmpeg

- Version: `n8.1.2-34-g9b6c8969e0-20260807`
- Windows build: BtbN FFmpeg Builds, `win64-gpl-8.1`
- Source archive: `ffmpeg-n8.1.2-34-g9b6c8969e0-win64-gpl-8.1.zip`
- Archive SHA-256: `1555D35C6D6C747F152CB7C2F8B2E8CD5978A12AECD1E4863AD59438BCEF9492`
- `ffmpeg.exe` SHA-256: `FA142EBDE7643DF62FBF6B45161AD15111CA89A36B41373F058F73476E14F6D0`
- `ffprobe.exe` SHA-256: `E7F564AE34449A95912EF92D13CEAB91820C93706EE23EA04BCC50F527D289B1`
- Project: https://ffmpeg.org/
- Build project: https://github.com/BtbN/FFmpeg-Builds

This build is distributed under GPL v3 or later. The corresponding source and build scripts are available from the projects above. A copy of the GNU General Public License is available at https://www.gnu.org/licenses/gpl-3.0.html.

## ONNX Runtime and ort

Roto Now uses ONNX Runtime through pinned `ort` 2.0.0-rc.12. See https://github.com/pykeio/ort and https://github.com/microsoft/onnxruntime for their source and license terms.

## BiRefNet General Lite model

- Bundled file: `birefnet-general-lite.onnx`
- SHA-256: `5600024376F572A557870A5EB0AFB1E5961636BEF4E1E22132025467D0F03333`
- Bundled mixed-precision file: `birefnet-general-lite-fp16.onnx`
- Mixed-precision SHA-256: `311CFD8088EE71224BA0687B00DFAD1ED28FC05AAE0CE64E87965CC3D4B29D6A`
- Distribution source: https://github.com/danielgatis/rembg/releases
- Upstream project: https://github.com/ZhengPeng7/BiRefNet

The model is distributed under the MIT license. License and source information are available from the projects above.
