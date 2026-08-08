$ErrorActionPreference = "Stop"

$archiveUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-07-13-13/ffmpeg-n8.1.2-34-g9b6c8969e0-win64-gpl-8.1.zip"
$archiveSha256 = "1555D35C6D6C747F152CB7C2F8B2E8CD5978A12AECD1E4863AD59438BCEF9492"
$binaryHashes = @{
    "ffmpeg.exe" = "FA142EBDE7643DF62FBF6B45161AD15111CA89A36B41373F058F73476E14F6D0"
    "ffprobe.exe" = "E7F564AE34449A95912EF92D13CEAB91820C93706EE23EA04BCC50F527D289B1"
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$binaryRoot = Join-Path $projectRoot "src-tauri\bin"
$downloadRoot = Join-Path $projectRoot ".toolchains\downloads"
$archivePath = Join-Path $downloadRoot "ffmpeg-8.1.2-win64-gpl.zip"
$extractRoot = Join-Path $downloadRoot "ffmpeg-8.1.2-extracted"

New-Item -ItemType Directory -Force -Path $binaryRoot, $downloadRoot | Out-Null

$ready = $true
foreach ($entry in $binaryHashes.GetEnumerator()) {
    $path = Join-Path $binaryRoot $entry.Key
    if (!(Test-Path -LiteralPath $path) -or (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Value) {
        $ready = $false
    }
}
if ($ready) {
    Write-Host "Pinned FFmpeg 8.1.2 binaries are already ready."
    exit 0
}

Invoke-WebRequest -Uri $archiveUrl -OutFile $archivePath -UseBasicParsing
if ((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash -ne $archiveSha256) {
    throw "FFmpeg archive checksum verification failed."
}

if (Test-Path -LiteralPath $extractRoot) { Remove-Item -LiteralPath $extractRoot -Recurse -Force }
Expand-Archive -LiteralPath $archivePath -DestinationPath $extractRoot
foreach ($entry in $binaryHashes.GetEnumerator()) {
    $source = Get-ChildItem -LiteralPath $extractRoot -Recurse -File -Filter $entry.Key | Where-Object { $_.Directory.Name -eq "bin" } | Select-Object -First 1
    if (!$source) { throw "$($entry.Key) was not found in the pinned FFmpeg archive." }
    Copy-Item -LiteralPath $source.FullName -Destination (Join-Path $binaryRoot $entry.Key) -Force
    if ((Get-FileHash -LiteralPath (Join-Path $binaryRoot $entry.Key) -Algorithm SHA256).Hash -ne $entry.Value) {
        throw "$($entry.Key) checksum verification failed."
    }
}

Remove-Item -LiteralPath $extractRoot -Recurse -Force
Write-Host "Pinned FFmpeg 8.1.2 binaries are ready for packaging."
