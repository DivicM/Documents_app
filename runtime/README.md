# ONNX Runtime

`onnxruntime.dll` is not committed: it is 11 MB and identical for everyone.

Download it once per machine:

1. Get `onnxruntime-win-x64-<version>.zip` from
   https://github.com/microsoft/onnxruntime/releases
2. Extract `lib/onnxruntime.dll` into this directory.

Version 1.20.1 is what `ort 2.0.0-rc.13` expects.

The app loads this library at startup (`load-dynamic`) rather than linking it
statically, so it ships alongside the executable in the installer. Nothing is
downloaded at runtime.
