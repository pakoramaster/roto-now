$ErrorActionPreference = "Stop"

$modelUrl = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx"
$modelSha256 = "5600024376F572A557870A5EB0AFB1E5961636BEF4E1E22132025467D0F03333"
$projectRoot = Split-Path -Parent $PSScriptRoot
$modelRoot = Join-Path $projectRoot "src-tauri\models"
$modelPath = Join-Path $modelRoot "birefnet-general-lite.onnx"
$partialPath = "$modelPath.part"

New-Item -ItemType Directory -Force -Path $modelRoot | Out-Null
if ((Test-Path -LiteralPath $modelPath) -and (Get-FileHash -LiteralPath $modelPath -Algorithm SHA256).Hash -eq $modelSha256) {
    Write-Host "Pinned General Lite model is already ready."
    exit 0
}

Invoke-WebRequest -Uri $modelUrl -OutFile $partialPath -UseBasicParsing
if ((Get-FileHash -LiteralPath $partialPath -Algorithm SHA256).Hash -ne $modelSha256) {
    Remove-Item -LiteralPath $partialPath -Force
    throw "General Lite model checksum verification failed."
}
Move-Item -LiteralPath $partialPath -Destination $modelPath -Force
Write-Host "Pinned General Lite model is ready for packaging."
