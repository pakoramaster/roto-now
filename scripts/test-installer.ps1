param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,
    [string]$SandboxRoot,
    [switch]$RequireAllModels
)

$ErrorActionPreference = "Stop"
$runnerTempValue = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
if ([string]::IsNullOrWhiteSpace($SandboxRoot)) {
    $SandboxRoot = Join-Path $runnerTempValue "roto-now-installer-smoke"
}
$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$runnerTemp = (Resolve-Path -LiteralPath $runnerTempValue).Path.TrimEnd('\')
$sandboxParent = Split-Path -Parent $SandboxRoot
New-Item -ItemType Directory -Force -Path $sandboxParent | Out-Null
$resolvedParent = (Resolve-Path -LiteralPath $sandboxParent).Path.TrimEnd('\')
if (!$resolvedParent.StartsWith($runnerTemp, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Installer smoke-test directory must remain under RUNNER_TEMP."
}
$installRoot = Join-Path $SandboxRoot "installed"
$uninstallerPath = $null
$expectedVersion = (Get-Content (Join-Path (Split-Path -Parent $PSScriptRoot) "package.json") -Raw | ConvertFrom-Json).version

function Stop-SandboxApplication {
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $applications = @(
            Get-Process -Name "roto-now" -ErrorAction SilentlyContinue |
                Where-Object { $_.Path -and $_.Path.StartsWith($installRoot, [StringComparison]::OrdinalIgnoreCase) }
        )
        if ($applications.Count -eq 0) {
            return
        }
        $applications | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 100
    }
}

function Remove-Sandbox {
    Stop-SandboxApplication
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        try {
            if (Test-Path -LiteralPath $SandboxRoot) {
                Remove-Item -LiteralPath $SandboxRoot -Recurse -Force
            }
            return
        } catch {
            Stop-SandboxApplication
            Start-Sleep -Milliseconds 200
        }
    }
    Remove-Item -LiteralPath $SandboxRoot -Recurse -Force
}

Remove-Sandbox
New-Item -ItemType Directory -Force -Path $SandboxRoot | Out-Null

function Invoke-Installer {
    $process = Start-Process -FilePath $installer -ArgumentList @("/S", "/D=$installRoot") -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) {
        throw "Silent installer exited with code $($process.ExitCode)."
    }
}

try {
    Invoke-Installer
    Stop-SandboxApplication
    Invoke-Installer
    Stop-SandboxApplication

    $uninstaller = Get-ChildItem -LiteralPath $installRoot -File -Filter "uninstall.exe" | Select-Object -First 1
    if (!$uninstaller) { throw "NSIS uninstaller was not found." }
    $uninstallerPath = $uninstaller.FullName
    $application = Get-ChildItem -LiteralPath $installRoot -Recurse -File -Filter "roto-now.exe" | Select-Object -First 1
    if (!$application) { throw "Installed application executable was not found." }
    if (!$application.VersionInfo.ProductVersion.StartsWith($expectedVersion, [StringComparison]::Ordinal)) {
        throw "Installed application version '$($application.VersionInfo.ProductVersion)' does not match '$expectedVersion'."
    }
    $requiredAssets = @(
        "bin\ffmpeg.exe",
        "bin\ffprobe.exe",
        "models\birefnet-general-lite.onnx",
        "models\birefnet-general-lite-fp16.onnx"
    )
    if ($RequireAllModels) {
        $requiredAssets += @(
            "models\birefnet-general.onnx",
            "models\birefnet-toonout-fp16.onnx"
        )
    }
    foreach ($relativePath in $requiredAssets) {
        if (!(Test-Path -LiteralPath (Join-Path $installRoot $relativePath) -PathType Leaf)) {
            throw "Installed bundle asset is missing: $relativePath"
        }
    }

    $process = Start-Process -FilePath $uninstallerPath -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) {
        throw "Silent uninstaller exited with code $($process.ExitCode)."
    }
    if (Test-Path -LiteralPath $application.FullName) {
        throw "Uninstall left the application executable behind."
    }
    Write-Host "Installer install, repair, bundle-content, and uninstall smoke checks passed."
} finally {
    Stop-SandboxApplication
    if ($uninstallerPath -and (Test-Path -LiteralPath $uninstallerPath)) {
        Start-Process -FilePath $uninstallerPath -ArgumentList "/S" -Wait -WindowStyle Hidden -ErrorAction SilentlyContinue
    }
    Remove-Sandbox
}
