$projectRoot = Split-Path -Parent $PSScriptRoot
$env:CARGO_HOME = Join-Path $projectRoot ".toolchains\cargo"
$env:RUSTUP_HOME = Join-Path $projectRoot ".toolchains\rustup"
$env:Path = "$(Join-Path $env:CARGO_HOME 'bin');$env:Path"

Set-Location -LiteralPath $projectRoot
& (Join-Path $PSScriptRoot "fetch-ffmpeg.ps1")
& (Join-Path $PSScriptRoot "fetch-general-lite.ps1")
& npm.cmd run tauri dev
