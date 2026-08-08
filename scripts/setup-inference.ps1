$projectRoot = Split-Path -Parent $PSScriptRoot
$python = "C:\Users\hamza\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe"
$environment = Join-Path $projectRoot ".python-env"
$requirements = Join-Path $projectRoot "backend\requirements.txt"
$env:PIP_CACHE_DIR = Join-Path $environment "pip-cache"

if (-not (Test-Path -LiteralPath $python)) {
    throw "Python 3.12 was not found. Install Python 3.12 and update this script's `$python path."
}

if (-not (Test-Path -LiteralPath (Join-Path $environment "Scripts\python.exe"))) {
    & $python -m venv $environment
}

& (Join-Path $environment "Scripts\python.exe") -m pip install --disable-pip-version-check --progress-bar off --timeout 30 -r $requirements
