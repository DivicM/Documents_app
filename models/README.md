# Models

Both models are committed to the repository; nothing here needs downloading.

| File | Size | Purpose | Licence |
|---|---|---|---|
| `face_detection_yunet_2023mar.onnx` | 227 KB | Face detection and five landmarks | MIT (`LICENSE-yunet.txt`) |
| `u2netp.onnx` | 4.4 MB | Background segmentation | Apache-2.0 |

`u2netp` is the lightweight U²-Net, from
<https://github.com/xuebinqin/U-2-Net>, distributed as ONNX by
[rembg](https://github.com/danielgatis/rembg). Verify it with:

```powershell
(Get-FileHash models\u2netp.onnx -Algorithm SHA256).Hash
# 31ae7018f5d1bd6b869d1ec3e6ec52faead045b8f5fca562e4ba1e998718a3e1
```

This differs from the stock rembg download
(`309c8469…`) only in having its input height and width declared as dynamic
axes; the weights are untouched. The app runs it at 320x320 regardless — see
below.

## Why 320 and not more

U2NETP is fully convolutional, so it will accept a larger input, and 640x640
runs comfortably within the frame budget (about 400ms on an RTX 3050 Ti against
110ms at 320). It is nevertheless worse: measured on a test portrait, 640 kept
the head solid but broke the torso into a patchy, half-transparent mask, because
the network was trained at 320 and loses confidence away from it.

The mask is therefore produced at 320 and magnified by sampling it bilinearly
(`composite_background` in `crates/domain/src/mask.rs`). Nearest-neighbour
sampling was what made the enlargement show as blocky stair-steps along the
hairline.

Only `runtime\onnxruntime.dll` is fetched per machine, by `.\fetch-models.ps1`.

## Why not BiRefNet

The first version used `birefnet_lite_fp16.onnx` (109 MB, 1024×1024, 16446
nodes). It produced a slightly cleaner edge but took about seven seconds per
pass on a laptop GPU. Windows resets a graphics driver whose single call exceeds
the TDR timeout — two seconds by default — so the second and every later run hit
`887A0006 (GPU will not respond to more commands)` and cost roughly forty
seconds recovering.

`u2netp` runs the same photo in about 85 ms, measured on an RTX 3050 Ti, with no
driver reset. Its edge is marginally softer, which the threshold slider and the
mask brush already exist to correct.

Both models are bundled into the installer. The application makes no network
calls.
