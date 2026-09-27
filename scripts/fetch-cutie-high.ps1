$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content (Join-Path $projectRoot "src-tauri\model-manifest.json") -Raw | ConvertFrom-Json
$asset = $manifest.cutie
$modelRoot = Join-Path $projectRoot ".models"
$destination = Join-Path $modelRoot $asset.destination
$archive = Join-Path $modelRoot "$($asset.destination).zip"
$partial = "$archive.part"
$staging = Join-Path $modelRoot "$($asset.destination)-fetching"
function Test-Cutie([string]$Root) { if (!(Test-Path -LiteralPath $Root -PathType Container)) { return $false }; foreach ($member in $asset.archiveMembers) { $path = Join-Path $Root $member.path; if (!(Test-Path -LiteralPath $path -PathType Leaf) -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $member.sha256) { return $false } }; return $true }
New-Item -ItemType Directory -Force -Path $modelRoot | Out-Null
if (Test-Cutie $destination) { Write-Host "Pinned Cutie High Detail model is ready for development."; return }
$localSource = Join-Path $projectRoot "src-tauri\models\$($asset.destination)"
if (Test-Cutie $localSource) { Copy-Item -LiteralPath $localSource -Destination $destination -Recurse -Force; Write-Host "Pinned Cutie High Detail model was copied from the verified local cache."; return }
Invoke-WebRequest -Uri $asset.url -OutFile $partial -UseBasicParsing
if ((Get-Item -LiteralPath $partial).Length -ne [long]$asset.size -or (Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ne $asset.sha256) { Remove-Item -LiteralPath $partial -Force; throw "Cutie High Detail archive failed checksum verification." }
Move-Item -LiteralPath $partial -Destination $archive -Force
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
try { Expand-Archive -LiteralPath $archive -DestinationPath $staging -Force; if (!(Test-Cutie $staging)) { throw "Cutie High Detail files failed checksum verification." }; if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Recurse -Force }; Move-Item -LiteralPath $staging -Destination $destination }
finally { if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }; Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue }
Write-Host "Pinned Cutie High Detail model is ready for development."
