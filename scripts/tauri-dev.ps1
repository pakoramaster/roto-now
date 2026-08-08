$projectRoot = Split-Path -Parent $PSScriptRoot
$env:CARGO_HOME = Join-Path $projectRoot ".toolchains\cargo"
$env:RUSTUP_HOME = Join-Path $projectRoot ".toolchains\rustup"
$env:Path = "$(Join-Path $env:CARGO_HOME 'bin');$env:Path"

Set-Location -LiteralPath $projectRoot
& npm.cmd run tauri dev
