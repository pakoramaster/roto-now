$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$modelRoot = Join-Path $projectRoot "src-tauri\models"
$env:CARGO_HOME = Join-Path $projectRoot ".toolchains\cargo"
$env:RUSTUP_HOME = Join-Path $projectRoot ".toolchains\rustup"
$env:Path = "$(Join-Path $env:CARGO_HOME 'bin');$env:Path"
$sources = @{
    "birefnet-general.onnx" = @{
        Path = Join-Path $projectRoot ".models\rembg\birefnet-general.onnx"
        Sha256 = "58F621F00F5D756097615970A88A791584600DCF7C45B18A0A6267535A1EBD3C"
    }
    "birefnet-toonout-fp16.onnx" = @{
        Path = Join-Path $projectRoot ".models\toonout\birefnet-toonout-fp16.onnx"
        Sha256 = "213A8A98EE426EF8F02D247EB5A5A9889359E37C2E1E7E31E282D61034D08A83"
    }
}

New-Item -ItemType Directory -Force -Path $modelRoot | Out-Null
foreach ($entry in $sources.GetEnumerator()) {
    $source = $entry.Value.Path
    $destination = Join-Path $modelRoot $entry.Key
    if (!(Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Local model is missing: $source"
    }
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $entry.Value.Sha256) {
        throw "Local model failed checksum verification: $source"
    }
    if (!(Test-Path -LiteralPath $destination -PathType Leaf) -or
        (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $entry.Value.Sha256) {
        Copy-Item -LiteralPath $source -Destination $destination -Force
    }
}

& (Join-Path $PSScriptRoot "fetch-ffmpeg.ps1")
& (Join-Path $PSScriptRoot "fetch-general-lite.ps1")
& (Join-Path $PSScriptRoot "verify-release.ps1") -RequireBundleAssets -RequireAllModels

Push-Location $projectRoot
try {
    npm.cmd run tauri -- build --config src-tauri/tauri.all-models.conf.json
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri all-model installer build failed."
    }
} finally {
    Pop-Location
}

$installer = Get-ChildItem (Join-Path $projectRoot "src-tauri\target\release\bundle\nsis") -File -Filter "*.exe" |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (!$installer) {
    throw "The NSIS installer was not produced."
}
Write-Host "All-model installer ready: $($installer.FullName)"
