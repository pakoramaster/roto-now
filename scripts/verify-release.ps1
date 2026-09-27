param([switch]$RequireBundleAssets)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$package = Get-Content (Join-Path $projectRoot "package.json") -Raw | ConvertFrom-Json
$packageLockText = Get-Content (Join-Path $projectRoot "package-lock.json") -Raw
$tauriConfig = Get-Content (Join-Path $projectRoot "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$capability = Get-Content (Join-Path $projectRoot "src-tauri\capabilities\default.json") -Raw | ConvertFrom-Json
$cargoToml = Get-Content (Join-Path $projectRoot "src-tauri\Cargo.toml") -Raw
$cargoLock = Get-Content (Join-Path $projectRoot "src-tauri\Cargo.lock") -Raw
$modelManifest = Get-Content (Join-Path $projectRoot "src-tauri\model-manifest.json") -Raw | ConvertFrom-Json

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
if ($tauriConfig.bundle.createUpdaterArtifacts -ne $true) { throw "Signed updater artifacts must be enabled." }
if (!$tauriConfig.bundle.windows.nsis.installerHooks) { throw "The NSIS required-model hook must be configured." }
if (!$tauriConfig.plugins.updater.pubkey -or $tauriConfig.plugins.updater.pubkey -eq "UPDATER_PUBLIC_KEY") { throw "The updater public key must be configured." }
if (@($tauriConfig.plugins.updater.endpoints).Count -ne 1 -or $tauriConfig.plugins.updater.endpoints[0] -notmatch 'github\.com/pakoramaster/roto-now/releases/latest/download/latest\.json') { throw "The updater must target the latest stable GitHub Release." }
$bundledModels = @($tauriConfig.bundle.resources | Where-Object { $_ -like 'models/*' })
if ($bundledModels.Count -ne 0) { throw "Model weights must not be embedded in the installer." }
$requiredInstallerResources = @(
    "model-manifest.json",
    "windows/install-required-models.ps1"
)
foreach ($resource in $requiredInstallerResources) {
    if (@($tauriConfig.bundle.resources) -notcontains $resource) {
        throw "Required installer resource is not bundled: $resource"
    }
}

$general = @($modelManifest.models | Where-Object id -eq 'generalLite')
$maximum = @($modelManifest.models | Where-Object id -eq 'general')
$anime = @($modelManifest.models | Where-Object id -eq 'anime')
if ($general.Count -ne 1 -or $general[0].role -ne 'required' -or $general[0].destination -ne 'birefnet-general-lite-fp16.onnx') { throw "General FP16 must be the single required General asset." }
if ($maximum.Count -ne 1 -or $maximum[0].role -ne 'optional' -or $anime.Count -ne 1 -or $anime[0].role -ne 'optional') { throw "Maximum and Anime must remain optional assets." }
if ($modelManifest.cutie.role -ne 'required' -or @($modelManifest.cutie.archiveMembers).Count -ne 4) { throw "Cutie High Detail must be required with four pinned archive members." }
foreach ($asset in @($modelManifest.models) + @($modelManifest.cutie)) {
    if ($asset.sha256 -notmatch '^[0-9a-f]{64}$' -or [long]$asset.size -le 0 -or !$asset.url -or !$asset.destination -or !$asset.version) { throw "Model manifest asset '$($asset.id)' is incomplete." }
}

if ($RequireBundleAssets) {
    $assets = @{
        "src-tauri\bin\ffmpeg.exe" = "FA142EBDE7643DF62FBF6B45161AD15111CA89A36B41373F058F73476E14F6D0"
        "src-tauri\bin\ffprobe.exe" = "E7F564AE34449A95912EF92D13CEAB91820C93706EE23EA04BCC50F527D289B1"
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

Write-Host "Release configuration verified for Roto Now $version."
