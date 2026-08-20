param(
    [switch]$RequireBundleAssets,
    [switch]$RequireAllModels
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$package = Get-Content (Join-Path $projectRoot "package.json") -Raw | ConvertFrom-Json
$packageLockText = Get-Content (Join-Path $projectRoot "package-lock.json") -Raw
$tauriConfig = Get-Content (Join-Path $projectRoot "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$capability = Get-Content (Join-Path $projectRoot "src-tauri\capabilities\default.json") -Raw | ConvertFrom-Json
$cargoToml = Get-Content (Join-Path $projectRoot "src-tauri\Cargo.toml") -Raw
$cargoLock = Get-Content (Join-Path $projectRoot "src-tauri\Cargo.lock") -Raw

if ($package.version -notmatch '^\d+\.\d+\.\d+$') {
    throw "package.json must contain a stable semantic version."
}
$version = $package.version
$packageLockVersions = [regex]::Matches($packageLockText, '"version"\s*:\s*"([^"]+)"')
if ($packageLockVersions.Count -lt 2) {
    throw "package-lock.json does not contain the expected root versions."
}
$cargoVersion = [regex]::Match($cargoToml, '(?m)^version = "([^"]+)"').Groups[1].Value
$lockedVersion = [regex]::Match($cargoLock, '(?ms)\[\[package\]\]\s+name = "roto-now"\s+version = "([^"]+)"').Groups[1].Value
$versions = @{
    "package-lock.json" = $packageLockVersions[0].Groups[1].Value
    "package-lock root package" = $packageLockVersions[1].Groups[1].Value
    "src-tauri/Cargo.toml" = $cargoVersion
    "src-tauri/Cargo.lock" = $lockedVersion
    "src-tauri/tauri.conf.json" = $tauriConfig.version
}
foreach ($entry in $versions.GetEnumerator()) {
    if ($entry.Value -ne $version) {
        throw "$($entry.Key) version '$($entry.Value)' does not match package.json '$version'."
    }
}

if (!$tauriConfig.app.security.csp) {
    throw "A production Content Security Policy is required."
}
if (@($tauriConfig.app.security.assetProtocol.scope) -contains "**") {
    throw "The asset protocol must not grant unrestricted filesystem access."
}
if (@($capability.permissions) -contains "core:default") {
    throw "The desktop capability must grant only the core APIs used by the frontend."
}
if ($tauriConfig.bundle.windows.allowDowngrades -ne $false) {
    throw "Windows installer downgrades must be disabled."
}

if ($RequireBundleAssets) {
    $assets = @{
        "src-tauri\bin\ffmpeg.exe" = "FA142EBDE7643DF62FBF6B45161AD15111CA89A36B41373F058F73476E14F6D0"
        "src-tauri\bin\ffprobe.exe" = "E7F564AE34449A95912EF92D13CEAB91820C93706EE23EA04BCC50F527D289B1"
        "src-tauri\models\birefnet-general-lite.onnx" = "5600024376F572A557870A5EB0AFB1E5961636BEF4E1E22132025467D0F03333"
        "src-tauri\models\birefnet-general-lite-fp16.onnx" = "311CFD8088EE71224BA0687B00DFAD1ED28FC05AAE0CE64E87965CC3D4B29D6A"
    }
    foreach ($entry in $assets.GetEnumerator()) {
        $path = Join-Path $projectRoot $entry.Key
        if (!(Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Required bundle asset is missing: $($entry.Key)"
        }
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Value) {
            throw "Required bundle asset failed checksum verification: $($entry.Key)"
        }
    }
}

if ($RequireAllModels) {
    $models = @{
        "src-tauri\models\birefnet-general.onnx" = "58F621F00F5D756097615970A88A791584600DCF7C45B18A0A6267535A1EBD3C"
        "src-tauri\models\birefnet-toonout-fp16.onnx" = "213A8A98EE426EF8F02D247EB5A5A9889359E37C2E1E7E31E282D61034D08A83"
    }
    foreach ($entry in $models.GetEnumerator()) {
        $path = Join-Path $projectRoot $entry.Key
        if (!(Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Required all-model bundle asset is missing: $($entry.Key)"
        }
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.Value) {
            throw "Required all-model bundle asset failed checksum verification: $($entry.Key)"
        }
    }
}

Write-Host "Release configuration verified for Roto Now $version."
