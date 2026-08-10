# Downloads the files too large to keep in git: the segmentation model and the
# ONNX Runtime library. Run once per machine, from the repository root.
#
#   .\fetch-models.ps1
#
# This is a development step. The built application never downloads anything.

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Get-IfMissing {
    param(
        [string]$Path,
        [string]$Url,
        [string]$Sha256,
        [string]$Description
    )

    if (Test-Path $Path) {
        if ($Sha256) {
            $actual = (Get-FileHash $Path -Algorithm SHA256).Hash.ToLower()
            if ($actual -eq $Sha256.ToLower()) {
                Write-Host "ok       $Path" -ForegroundColor DarkGray
                return
            }
            Write-Host "checksum mismatch, re-downloading $Path" -ForegroundColor Yellow
            Remove-Item $Path -Force
        } else {
            Write-Host "ok       $Path" -ForegroundColor DarkGray
            return
        }
    }

    $dir = Split-Path $Path -Parent
    if ($dir -and -not (Test-Path $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }

    Write-Host "fetching $Description..." -ForegroundColor Cyan
    # Progress rendering makes Invoke-WebRequest far slower on large files.
    $previous = $ProgressPreference
    $ProgressPreference = "SilentlyContinue"
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Path -UseBasicParsing
    } finally {
        $ProgressPreference = $previous
    }

    if ($Sha256) {
        $actual = (Get-FileHash $Path -Algorithm SHA256).Hash.ToLower()
        if ($actual -ne $Sha256.ToLower()) {
            Remove-Item $Path -Force
            Write-Host "checksum mismatch for $Path; the download was discarded." -ForegroundColor Red
            exit 1
        }
    }
    Write-Host "done     $Path" -ForegroundColor Green
}

# Both models are committed to the repository, so only the runtime is fetched.

# ONNX Runtime ships as a NuGet package; the DirectML build is the one that can
# use the GPU, falling back to CPU where there isn't one.
if (-not (Test-Path "runtime\onnxruntime.dll")) {
    Write-Host "fetching ONNX Runtime with DirectML (16 MB)..." -ForegroundColor Cyan
    $tmp = Join-Path $env:TEMP "ort-dml.zip"
    $extract = Join-Path $env:TEMP "ort-dml-extract"
    $previous = $ProgressPreference
    $ProgressPreference = "SilentlyContinue"
    try {
        Invoke-WebRequest `
            -Uri "https://www.nuget.org/api/v2/package/Microsoft.ML.OnnxRuntime.DirectML/1.20.1" `
            -OutFile $tmp -UseBasicParsing
    } finally {
        $ProgressPreference = $previous
    }

    if (Test-Path $extract) { Remove-Item $extract -Recurse -Force }
    Expand-Archive $tmp -DestinationPath $extract -Force

    $dll = Join-Path $extract "runtimes\win-x64\native\onnxruntime.dll"
    if (-not (Test-Path $dll)) {
        Write-Host "onnxruntime.dll not found in the package" -ForegroundColor Red
        exit 1
    }
    New-Item -ItemType Directory -Force -Path "runtime" | Out-Null
    Copy-Item $dll "runtime\onnxruntime.dll" -Force
    Remove-Item $tmp, $extract -Recurse -Force
    Write-Host "done     runtime\onnxruntime.dll" -ForegroundColor Green
} else {
    Write-Host "ok       runtime\onnxruntime.dll" -ForegroundColor DarkGray
}

Write-Host "`nAll files present." -ForegroundColor Green
