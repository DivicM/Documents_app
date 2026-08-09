# Models

`face_detection_yunet_2023mar.onnx` (227 KB) is committed. `birefnet_lite_fp16.onnx`
is not: at 109 MB it exceeds GitHub's 100 MB file limit.

Download it once per machine, from the repository root:

```powershell
.\fetch-models.ps1
```

Or by hand:

```
https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/main/onnx/model_fp16.onnx
```

saved as `models/birefnet_lite_fp16.onnx`. Verify it with:

```powershell
(Get-FileHash models\birefnet_lite_fp16.onnx -Algorithm SHA256).Hash
# d39b897ceb16ae654c1731f3dba0cf9b368d9cae74b5a57459b455cc8bfec402
```

Both models are permissively licensed: YuNet is MIT (see `LICENSE-yunet.txt`),
BiRefNet is MIT. They are bundled into the installer, never downloaded by the
app itself — the download above is a development step, and the built
application makes no network calls.
