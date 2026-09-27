param([Parameter(Mandatory = $true)][string]$ManifestPath, [Parameter(Mandatory = $true)][string]$ModelRoot)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
Add-Type -AssemblyName System.Net.Http
function Get-Hash([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Test-Asset([string]$Path, [long]$Size, [string]$Sha256) { (Test-Path -LiteralPath $Path -PathType Leaf) -and ((Get-Item -LiteralPath $Path).Length -eq $Size) -and ((Get-Hash $Path) -eq $Sha256) }
function Receive-Asset($Asset, [string]$Destination) {
  $part = "$Destination.part"; $existing = if (Test-Path -LiteralPath $part) { (Get-Item -LiteralPath $part).Length } else { 0 }
  if ($existing -gt [long]$Asset.size) { Remove-Item -LiteralPath $part -Force; $existing = 0 }
  if ($existing -eq [long]$Asset.size) { if ((Get-Hash $part) -eq $Asset.sha256) { return $part }; Remove-Item -LiteralPath $part -Force; $existing = 0 }
  Write-Host "Downloading $($Asset.name)"
  $handler = [System.Net.Http.HttpClientHandler]::new(); $client = [System.Net.Http.HttpClient]::new($handler); $client.DefaultRequestHeaders.UserAgent.ParseAdd("RotoNow-Installer/1")
  try {
    $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Get, [string]$Asset.url)
    if ($existing -gt 0) { $request.Headers.Range = [System.Net.Http.Headers.RangeHeaderValue]::new($existing, $null) }
    $response = $client.SendAsync($request, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult(); $null = $response.EnsureSuccessStatusCode()
    $append = $existing -gt 0 -and [int]$response.StatusCode -eq 206; if (!$append) { $existing = 0 }; $mode = if ($append) { [IO.FileMode]::Append } else { [IO.FileMode]::Create }
    $output = [IO.File]::Open($part, $mode, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $input = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult(); try { $input.CopyTo($output) } finally { $input.Dispose() } } finally { $output.Dispose() }
  } finally { $client.Dispose(); $handler.Dispose() }
  if (!(Test-Asset $part ([long]$Asset.size) ([string]$Asset.sha256))) { Remove-Item -LiteralPath $part -Force -ErrorAction SilentlyContinue; throw "$($Asset.name) failed checksum verification." }
  $part
}
function Install-FileAsset($Asset) {
  $destination = Join-Path $ModelRoot $Asset.destination
  if (Test-Asset $destination ([long]$Asset.size) ([string]$Asset.sha256)) { Write-Output "$($Asset.name) is already installed; skipping."; return }
  $part = Receive-Asset $Asset $destination
  if (Test-Path -LiteralPath $destination) { $backup = "$destination.old"; Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue; [IO.File]::Replace($part, $destination, $backup, $true); Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue } else { [IO.File]::Move($part, $destination) }
  Write-Output "Installed $($Asset.name)."
}
function Test-Cutie($Asset, [string]$Root) { if (!(Test-Path -LiteralPath $Root -PathType Container)) { return $false }; foreach ($member in $Asset.archiveMembers) { $path = Join-Path $Root $member.path; if (!(Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Hash $path) -ne $member.sha256) { return $false } }; $true }
function Install-Cutie($Asset) {
  $destination = Join-Path $ModelRoot $Asset.destination
  if (Test-Cutie $Asset $destination) { Write-Output "$($Asset.name) is already installed; skipping."; return }
  $archive = Join-Path $ModelRoot "$($Asset.destination).zip"
  if (!(Test-Asset $archive ([long]$Asset.size) ([string]$Asset.sha256))) { $part = Receive-Asset $Asset $archive; if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }; [IO.File]::Move($part, $archive) }
  Add-Type -AssemblyName System.IO.Compression.FileSystem; $staging = Join-Path $ModelRoot "$($Asset.destination)-installing"
  if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }; New-Item -ItemType Directory -Path $staging -Force | Out-Null
  $zip = [IO.Compression.ZipFile]::OpenRead($archive)
  try { foreach ($member in $Asset.archiveMembers) { $entry = $zip.Entries | Where-Object { $_.FullName -eq $member.path } | Select-Object -First 1; if (!$entry -or [IO.Path]::GetFileName($entry.FullName) -ne $entry.FullName) { throw "Cutie archive is missing a safe $($member.path) entry." }; $path = Join-Path $staging $member.path; [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $path, $true); if ((Get-Hash $path) -ne $member.sha256) { throw "$($member.path) failed checksum verification." } } } finally { $zip.Dispose() }
  if (!(Test-Cutie $Asset $staging)) { throw "Cutie installation is incomplete." }
  $backup = "$destination.old"; if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }; if (Test-Path -LiteralPath $destination) { [IO.Directory]::Move($destination, $backup) }
  try { [IO.Directory]::Move($staging, $destination) } catch { if (Test-Path -LiteralPath $backup) { [IO.Directory]::Move($backup, $destination) }; throw }
  if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }; Write-Output "Installed $($Asset.name)."
}
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json; New-Item -ItemType Directory -Path $ModelRoot -Force | Out-Null
$general = $manifest.models | Where-Object { $_.id -eq "generalLite" -and $_.role -eq "required" }; if (!$general) { throw "Required General model is missing from the manifest." }; Install-FileAsset $general
if ($manifest.cutie.role -ne "required") { throw "Required Cutie model is missing from the manifest." }; Install-Cutie $manifest.cutie
