$ErrorActionPreference = "Stop"

$modelUrl = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx"
$modelSha256 = "5600024376F572A557870A5EB0AFB1E5961636BEF4E1E22132025467D0F03333"
$projectRoot = Split-Path -Parent $PSScriptRoot
$modelRoot = Join-Path $projectRoot "src-tauri\models"
$modelPath = Join-Path $modelRoot "birefnet-general-lite.onnx"
$partialPath = "$modelPath.part"

New-Item -ItemType Directory -Force -Path $modelRoot | Out-Null
if ((Test-Path -LiteralPath $modelPath) -and (Get-FileHash -LiteralPath $modelPath -Algorithm SHA256).Hash -eq $modelSha256) {
    Write-Host "Pinned General Lite FP32 model is already ready."
} else {
    Invoke-WebRequest -Uri $modelUrl -OutFile $partialPath -UseBasicParsing
    if ((Get-FileHash -LiteralPath $partialPath -Algorithm SHA256).Hash -ne $modelSha256) {
        Remove-Item -LiteralPath $partialPath -Force
        throw "General Lite model checksum verification failed."
    }
    Move-Item -LiteralPath $partialPath -Destination $modelPath -Force
}

$fp16Path = Join-Path $modelRoot "birefnet-general-lite-fp16.onnx"
$fp16Sha256 = "311CFD8088EE71224BA0687B00DFAD1ED28FC05AAE0CE64E87965CC3D4B29D6A"
if (!(Test-Path -LiteralPath $fp16Path) -or (Get-FileHash -LiteralPath $fp16Path -Algorithm SHA256).Hash -ne $fp16Sha256) {
    $python = Join-Path $projectRoot ".python-env\Scripts\python.exe"
    if (!(Test-Path -LiteralPath $python)) {
        throw "The project Python environment is required to create the FP16 bundle model."
    }
    & $python (Join-Path $PSScriptRoot "convert-general-lite-fp16.py")
    if ($LASTEXITCODE -ne 0 -or (Get-FileHash -LiteralPath $fp16Path -Algorithm SHA256).Hash -ne $fp16Sha256) {
        throw "FP16 General Lite conversion failed checksum verification."
    }
}
Write-Host "Pinned FP32 and FP16 General Lite models are ready for packaging."
