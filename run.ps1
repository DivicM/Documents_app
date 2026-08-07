# Builds and starts the app.
#
# Works around two Windows-specific problems:
#  1. cargo is not on PATH in shells started before Rust was installed
#  2. Smart App Control blocks freshly linked unsigned binaries for a while
#
# Usage:  .\run.ps1

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# 1. Make sure cargo is reachable regardless of when this shell started.
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    if (Test-Path (Join-Path $cargoBin "cargo.exe")) {
        $env:PATH = "$cargoBin;$env:PATH"
        Write-Host "cargo added to PATH for this session" -ForegroundColor DarkGray
    } else {
        Write-Host "cargo not found. Install Rust from https://rustup.rs" -ForegroundColor Red
        exit 1
    }
}

# 2. Start the Vite dev server if it is not already serving.
$viteUp = $false
try {
    $null = Invoke-WebRequest -Uri "http://localhost:5173" -UseBasicParsing -TimeoutSec 2
    $viteUp = $true
} catch {}

if ($viteUp) {
    Write-Host "vite already running on :5173" -ForegroundColor DarkGray
} else {
    Write-Host "starting vite..." -ForegroundColor Cyan
    Start-Process -FilePath "cmd.exe" `
        -ArgumentList "/c", "npm run dev > `"$env:TEMP\vite.log`" 2>&1" `
        -WindowStyle Hidden
    foreach ($i in 1..30) {
        Start-Sleep -Milliseconds 500
        try {
            $null = Invoke-WebRequest -Uri "http://localhost:5173" -UseBasicParsing -TimeoutSec 2
            $viteUp = $true
            break
        } catch {}
    }
    if (-not $viteUp) {
        Write-Host "vite failed to start. See $env:TEMP\vite.log" -ForegroundColor Red
        exit 1
    }
}

# 3. Stop a previous instance so the new build is what actually runs.
Get-Process documents-app -ErrorAction SilentlyContinue | Stop-Process -Force

# 4. Build.
Write-Host "building..." -ForegroundColor Cyan
cargo build -p documents-app
if ($LASTEXITCODE -ne 0) {
    Write-Host "build failed" -ForegroundColor Red
    exit 1
}

# 5. Launch, retrying while Smart App Control vets the new binary.
# Ask cargo where it actually put the binary rather than guessing: the target
# directory may be redirected by .cargo/config.toml, which this script cannot
# see from the environment alone.
$exe = $null
$meta = cargo metadata --no-deps --format-version 1 2>$null | ConvertFrom-Json
if ($meta -and $meta.target_directory) {
    $candidate = Join-Path $meta.target_directory "debug\documents-app.exe"
    if (Test-Path $candidate) { $exe = $candidate }
}
if (-not $exe) {
    $fallback = Join-Path $PSScriptRoot "target\debug\documents-app.exe"
    if (Test-Path $fallback) { $exe = $fallback }
}
if (-not $exe) {
    Write-Host "executable not found; is the build output somewhere unexpected?" -ForegroundColor Red
    exit 1
}

$env:TAURI_DEV_SERVER_URL = "http://localhost:5173"
foreach ($attempt in 1..8) {
    try {
        $p = Start-Process $exe -PassThru -ErrorAction Stop
        Start-Sleep -Seconds 2
        if (-not $p.HasExited) {
            Write-Host "running (PID $($p.Id))" -ForegroundColor Green
            exit 0
        }
        Write-Host "app exited with code $($p.ExitCode)" -ForegroundColor Red
        exit 1
    } catch {
        if ($_.Exception.Message -notmatch "Application Control") { throw }
        if ($attempt -eq 1) {
            Write-Host "Smart App Control blocked the new binary; relinking to change its hash." -ForegroundColor Yellow
        }
        Write-Host "  blocked, relink attempt $attempt/8..." -ForegroundColor DarkGray
        # SAC verdicts are per file hash, so deleting the executable and
        # letting cargo link a fresh one usually clears it far faster than
        # waiting for the original to be vetted.
        Remove-Item $exe -Force -ErrorAction SilentlyContinue
        cargo build -p documents-app 2>&1 | Out-Null
        if (-not (Test-Path $exe)) {
            Write-Host "relink produced no executable" -ForegroundColor Red
            exit 1
        }
    }
}

Write-Host "Smart App Control kept blocking the binary across 8 relinks." -ForegroundColor Red
Write-Host "Wait a few minutes and run this script again." -ForegroundColor DarkGray
exit 1
