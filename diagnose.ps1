# Reports what an installed build actually has on disk, and where.
#
# The application looks for `models/` and `runtime/` next to its executable
# (see resource_path in src-tauri/src/face.rs). This checks they are there,
# so a missing resource is not mistaken for a broken image.
#
# Run on the machine where the app is installed:
#   powershell -ExecutionPolicy Bypass -File .\diagnose.ps1

$app = "Fotografije za dokumente"

$roots = @(
    (Join-Path $env:LOCALAPPDATA "Programs\$app"),
    (Join-Path $env:ProgramFiles $app),
    (Join-Path ${env:ProgramFiles(x86)} $app)
) | Where-Object { $_ -and (Test-Path $_) }

if (-not $roots) {
    Write-Host "Application not found in the usual install locations." -ForegroundColor Red
    Write-Host "If it is installed elsewhere, run this from that folder." -ForegroundColor DarkGray
    exit 1
}

foreach ($root in $roots) {
    Write-Host "`n=== $root ===" -ForegroundColor Cyan

    $exe = Get-ChildItem $root -Filter "*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($exe) { Write-Host "executable : $($exe.Name)" } else { Write-Host "executable : NOT FOUND" -ForegroundColor Red }

    # These are the two paths resource_path() checks, relative to the exe.
    $needed = @{
        "models\face_detection_yunet_2023mar.onnx" = "face detection model"
        "models\u2netp.onnx"                       = "segmentation model"
        "runtime\onnxruntime.dll"                  = "ONNX Runtime"
    }

    foreach ($rel in $needed.Keys | Sort-Object) {
        $p = Join-Path $root $rel
        if (Test-Path $p) {
            $kb = [math]::Round((Get-Item $p).Length / 1KB)
            Write-Host ("  OK      {0,-45} {1} KB" -f $rel, $kb) -ForegroundColor Green
        } else {
            Write-Host ("  MISSING {0,-45} ({1})" -f $rel, $needed[$rel]) -ForegroundColor Red
        }
    }

    # If they are missing, they may have been bundled into a subfolder instead.
    Write-Host "`n  any .onnx / onnxruntime.dll anywhere under this folder:" -ForegroundColor DarkGray
    $found = Get-ChildItem $root -Recurse -Include *.onnx, onnxruntime.dll -ErrorAction SilentlyContinue
    if ($found) {
        foreach ($f in $found) {
            Write-Host ("    {0}" -f $f.FullName.Substring($root.Length + 1)) -ForegroundColor DarkGray
        }
    } else {
        Write-Host "    none" -ForegroundColor Red
    }
}

Write-Host "`n=== WebView2 ===" -ForegroundColor Cyan
$wv = Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue
if (-not $wv) {
    $wv = Get-ItemProperty "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue
}
if ($wv) { Write-Host "installed, version $($wv.pv)" -ForegroundColor Green }
else { Write-Host "NOT DETECTED" -ForegroundColor Red }
