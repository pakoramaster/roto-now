param(
    [Parameter(Mandatory = $true)]
    [string]$TargetPath
)

$ErrorActionPreference = "Stop"
$certificatePath = $env:WINDOWS_SIGNING_CERTIFICATE_PATH
$certificatePassword = $env:WINDOWS_SIGNING_PASSWORD
if ([string]::IsNullOrWhiteSpace($certificatePath) -and [string]::IsNullOrWhiteSpace($certificatePassword)) {
    Write-Host "Signing secrets are not configured; leaving $TargetPath unsigned."
    exit 0
}
if ([string]::IsNullOrWhiteSpace($certificatePath) -or [string]::IsNullOrWhiteSpace($certificatePassword)) {
    throw "Both WINDOWS_SIGNING_CERTIFICATE_PATH and WINDOWS_SIGNING_PASSWORD are required."
}
if (!(Test-Path -LiteralPath $TargetPath -PathType Leaf)) {
    throw "Signing target does not exist: $TargetPath"
}
if (!(Test-Path -LiteralPath $certificatePath -PathType Leaf)) {
    throw "Signing certificate file does not exist."
}

$signTool = if ($env:SIGNTOOL_PATH) {
    Get-Item -LiteralPath $env:SIGNTOOL_PATH
} else {
    Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin" -Recurse -Filter signtool.exe |
        Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
}
if (!$signTool) {
    throw "SignTool was not found."
}

& $signTool.FullName sign /fd SHA256 /td SHA256 /tr https://timestamp.digicert.com /f $certificatePath /p $certificatePassword $TargetPath
if ($LASTEXITCODE -ne 0) {
    throw "Authenticode signing failed for $TargetPath."
}
