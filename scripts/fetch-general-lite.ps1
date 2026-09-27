$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content (Join-Path $projectRoot "src-tauri\model-manifest.json") -Raw | ConvertFrom-Json
$asset = $manifest.models | Where-Object id -eq "generalLite"
if (!$asset -or $asset.role -ne "required") { throw "Required General FP16 asset is missing from the model manifest." }
$destinationRoot = Join-Path $projectRoot ".models\rembg"
$destination = Join-Path $destinationRoot $asset.destination
$partial = "$destination.part"
New-Item -ItemType Directory -Force -Path $destinationRoot | Out-Null
function Test-General([string]$Path) { (Test-Path -LiteralPath $Path -PathType Leaf) -and ((Get-Item -LiteralPath $Path).Length -eq [long]$asset.size) -and ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -eq $asset.sha256) }
if (Test-General $destination) { Write-Host "Pinned General FP16 model is ready for development."; return }
$localSource = Join-Path $projectRoot "src-tauri\models\$($asset.destination)"
if (Test-General $localSource) { Copy-Item -LiteralPath $localSource -Destination $partial -Force } else { Invoke-WebRequest -Uri $asset.url -OutFile $partial -UseBasicParsing }
if (!(Test-General $partial)) { Remove-Item -LiteralPath $partial -Force -ErrorAction SilentlyContinue; throw "General FP16 model failed checksum verification." }
Move-Item -LiteralPath $partial -Destination $destination -Force
Write-Host "Pinned General FP16 model is ready for development."
