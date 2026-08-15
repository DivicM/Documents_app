# Fotografije za dokumente (Document Photos)

A desktop application for producing and printing photographs for identity documents (passports, ID cards, visas). It turns an ordinary photograph into a correctly framed image at the prescribed format and arranges multiple copies on a single sheet of photo paper.

The application runs **entirely offline**. Face detection and background removal happen locally, on the user's machine; no photograph ever leaves the device.

- **Platform:** Windows (printing uses Win32 GDI)
- **Version:** 1.0.0
- **UI language:** Croatian
- **Stack:** Tauri 2 + React 18 (TypeScript) + Rust

---

## Contents

1. [What the application does](#what-the-application-does)
2. [Features](#features)
3. [Architecture](#architecture)
4. [Data flow](#data-flow)
5. [Key design decisions](#key-design-decisions)
6. [Document formats (specs)](#document-formats-specs)
7. [IPC surface](#ipc-surface)
8. [Settings and configuration](#settings-and-configuration)
9. [Development and running](#development-and-running)
10. [Testing](#testing)
11. [Repository structure](#repository-structure)

---

## What the application does

The user moves through four steps (a wizard):

| Step | Name | What happens |
|------|------|--------------|
| 1 | **Choose image** (*Odabir slike*) | Load a photograph (drag & drop or file dialog) |
| 2 | **Format** (*Format*) | Pick a document format (35×45, 30×40, 2×2" …) or enter custom dimensions |
| 3 | **Editing** (*Obrada*) | Automatic face detection, cropping, exposure correction, background replacement |
| 4 | **Print** (*Ispis*) | Arrange N copies on a sheet, preview and print |

The result is a sheet of photo paper (10×15, 13×18 or A4) holding several identical photographs at the correct physical size, with faint grey guides for cutting.

---

## Features

### Face detection and cropping

- **Automatic face detection** with the YuNet model (ONNX). The model returns a face box and five landmarks (eyes, nose, mouth corners).
- **Chin and crown estimation.** A five-point detector cannot see either directly — the chin sits below the face box and the crown is under the hair. Both are estimated from the eye line and flagged `AUTO`, so the user can drag them.
- **Crop solver** (`solve_crop`) computes the crop so the head prints at the requested height in millimetres. The ratio between head height in pixels and in millimetres fixes the scale; the rest follows from the format's aspect ratio.
- **Head-tilt straightening** — the crop can be rotated to the eye line without resampling twice (rotation and crop in a single pass).
- When the required crop extends past the image edges, the application neither shifts the frame silently nor upscales. It reports an error **with numbers**: how many pixels are missing, and which head height would fit.

### Background removal and replacement

- **Segmentation** with the U²-Netp model (ONNX), producing an alpha mask of the subject.
- **DirectML (GPU) with automatic CPU fallback** — the same build runs on machines without a suitable GPU.
- **Edge threshold** — raising it trims a halo of background the model left attached; lowering it recovers hair the model cut away.
- **Manual correction brush** (add/subtract) with per-stroke undo. Strokes are kept as paths rather than painted pixels, so re-running the model does not discard hand corrections.
- **Background compositing** to a flat colour, sampling the mask bilinearly (the mask is smaller than the image because the model runs at 320×320).

### Image adjustments

- Exposure, contrast and white balance (`adjust.rs`).
- Work is done in floating point and quantised back to 8-bit **exactly once, at the end of the chain** — successive adjustments do not compound rounding error.
- Measurement of **clipping** and **sharpness** (Laplacian variance over the face region, not the whole image).

### Compliance checking

The `rules.rs` module validates a photograph against the selected spec:

- head height, head width, eye distance, horizontal centring
- head roll, resolution (DPI), sharpness, exposure, background uniformity

Results are **i18n keys plus parameters**, never finished prose — the frontend decides the wording. Rules a five-point detector **cannot** measure (e.g. "mouth closed") are reported as `NotChecked` rather than as passing. A green tick beside something the program never checked would be a promise it cannot keep.

### Sheet layout

- **Layout solver** computes how many copies fit on the paper at a given margin and gutter, tries both orientations and keeps whichever fits more.
- **Balanced rows** — six photos in a four-column grid do not come out as 4 + 2 (with a visible gap) but as 3 + 3.
- **Evenly distributed slack** — when centring, the sheet is divided into `k + 1` equal gaps, so the distance between neighbours equals the distance to the paper edge. `gutter_mm` acts as a floor, never a fixed value.
- **Mixed sheet** (`solve_mixed`) — several different formats on one sheet, shelf-packed, largest groups first.
- **Orientation lock**, **landscape frame** (45×35 instead of 35×45) and **turning the picture inside its frame** as independent settings.

### Printing

- **Paper size read from the driver**, not assumed. A photo printer is configured for one paper size, and printing a layout computed for a different one puts the photos off the edge.
- **Rasterisation at device resolution**, blitted 1:1. The driver is never asked to scale: "fit to page" lives in the driver-private part of `DEVMODE`, `dmScale` is widely ignored, and neither is portable.
- **Printable-area offset** — the origin the driver reports is the top-left of the *printable region*, not of the paper, so a layout computed in paper coordinates is shifted back by the unprintable margin.
- **Cut guides** — 22 % grey, 0.35 mm thick. The thickness is specified in millimetres rather than pixels because a one-pixel line is 0.085 mm at 300 dpi and disappears entirely on better printers.
- **Double-print guard.**

### Printer calibration

Every printer misses the requested size slightly, and differently along the paper-feed direction than along the print-head direction.

- Print a **calibration square**, measure it with a ruler, type in the actual dimensions.
- The correction is stored **separately for X and Y**, keyed by `(printer, paper size, borderless)` — because both the paper and the borderless mode change the transport path.
- The correction is applied to every length before it becomes pixels. The paper itself is never scaled, only its contents.

### Other

- **Presets** — named sets of settings. A preset remembers parameters only, never pixels and never a crop (the crop derives from the detected face and is meaningless on a different photograph).
- **Undo/redo** for editing.
- **Dark theme**, **text size** (50–200 %), **start maximised** — persisted settings.
- Editing works on a **downscaled working copy** (longest edge 1600 px) for speed; full resolution is used only for printing.
- **ONNX models preloaded** on a background thread at startup (~2 s that would otherwise be charged to the user's first action).

---

## Architecture

The project is a Cargo workspace with three library crates and one Tauri binary, plus the React frontend.

```
┌──────────────────────────────────────────────────────────────┐
│  React + TypeScript  (src/)                                  │
│  wizard, editing canvas, sheet preview, settings             │
└──────────────────────────┬───────────────────────────────────┘
                           │  Tauri IPC  (src/lib/ipc.ts)
                           │  JSON for parameters, raw bytes for pixels
┌──────────────────────────▼───────────────────────────────────┐
│  documents-app  (src-tauri/)                                 │
│  commands.rs · face.rs · background.rs · spec.rs · presets.rs│
│  thin layer: validate input, call, map errors                │
└───────┬───────────────────┬──────────────────┬───────────────┘
        │                   │                  │
┌───────▼────────┐ ┌────────▼────────┐ ┌───────▼──────────────┐
│  domain        │ │  vision         │ │  platform            │
│  pure logic    │ │  ONNX models    │ │  OS: printers, config│
│  no I/O        │ │  YuNet, U²-Netp │ │  Win32 GDI, TOML     │
└────────────────┘ └─────────────────┘ └──────────────────────┘
```

### `crates/domain` — pure logic

No images, no models, no UI, no operating system. **Everything works in millimetres**; conversion to pixels happens in exactly one place.

| Module | Responsibility |
|--------|----------------|
| `units.rs` | The single place where millimetres become pixels. A DPI mistake has exactly one place to hide. |
| `geometry.rs` | Points, rectangles, chin/crown estimation, crop solver |
| `layout.rs` | Arranging N copies on a sheet; single-size and mixed layouts |
| `spec.rs` | Loading and resolving document specs (with inheritance) from JSON |
| `rules.rs` | Validating a photograph against a spec |
| `mask.rs` | Alpha mask, brush strokes, threshold, background compositing |
| `adjust.rs` | Exposure, contrast, white balance, sharpness and clipping measurement |
| `resample.rs` | Lanczos3 resampling (and bilinear for the rotated crop) |
| `render.rs` | Turning a layout into a BGRA raster ready for the printer |

Why this is separated: the layout solver and the geometry can be tested **without an image, a printer or a UI**. This is also where most of the ~250 tests live.

### `crates/vision` — models

| Module | Responsibility |
|--------|----------------|
| `detect.rs` | Running YuNet through ONNX Runtime (640×640 input) |
| `decode.rs` | Decoding raw tensors into detections — **free of ONNX**, so the arithmetic can be tested without a model or a GPU |
| `segment.rs` | U²-Netp segmentation (320×320 input), DirectML → CPU |

`decode.rs` is deliberately split from `detect.rs`: the arithmetic that decides *where* a face lands is testable without a model.

### `crates/platform` — operating system

| Module | Responsibility |
|--------|----------------|
| `print.rs` | Print backend abstraction: what the device can do, and a raster at its own resolution |
| `print_win.rs` | Win32 GDI implementation (`StretchDIBits`) |
| `calibration.rs` | Per-printer corrections, TOML in `%APPDATA%` |
| `presets.rs` | Presets and persisted sheet settings |

### `src-tauri` — the IPC layer

Commands are **thin**: validate input, call `domain` or `platform`, map errors into something the UI can show. No geometry and no layout logic live here.

State kept on the Rust side: the detector instance, the segmenter instance, the last mask and the user's strokes.

### `src` — React frontend

`App.tsx` holds the wizard state; components are presentational. `lib/ipc.ts` is the only bridge to Rust, `lib/i18n.ts` holds all text, `lib/history.ts` the undo/redo.

---

## Data flow

```
photograph (JPEG/PNG)
   │
   │  the webview decodes it (it already has a decoder)
   ▼
RGBA pixels in a canvas
   │
   ├─→ working copy (max 1600 px)  ──→ detection, segmentation, preview
   │
   └─→ full resolution  ──────────────────────────┐
                                                  │
working copy:                                     │
   detect_face  → box + 5 landmarks               │
   estimate_head_anchors → chin, crown (AUTO)     │
   compute_crop → crop rectangle                  │
   segment_background → alpha mask                │
   validate_photo → check results                 │
   solve_layout → sheet layout                    │
                                                  │
printing:                                         ▼
   print_sheet(JSON header + full RGBA + mask)
      → tone adjustments
      → background compositing
      → Lanczos3 resample of the crop to print size
      → calibration + printable-area offset
      → BGRA raster at device DPI
      → StretchDIBits, 1:1 blit
```

**Order matters and is identical in the preview and at print time:** tone first, then background. Adjusting tone *after* replacement would shift the background colour the user chose.

### Moving pixels across IPC

Pixels **do not travel as a JSON array of numbers**. A 2000×1333 photograph is 10.7 MB, which as JSON becomes 32 MB of text costing roughly two seconds to encode and parse — far more than the 85 ms the segmentation itself takes.

A raw request body is used instead:

```
[u32 JSON length][JSON parameters][RGBA pixels][mask]
```

The lengths are known from the parameters, so no additional framing is needed. Length validation uses **checked arithmetic** (`checked_mul`): the dimensions come off the wire, and an overflow could admit a short buffer that the renderer would then read past.

---

## Key design decisions

**Everything in millimetres, pixels only at the end.** The solver knows nothing about DPI, pixels or printers. The same code draws a 96 dpi preview and a 600 dpi sheet — it is simply handed a different DPI. That is what keeps the screen and the paper in agreement.

**Specs are data, not code.** Document formats are JSON with inheritance (`inherits`). Import, background removal, layout and printing are the same code whichever spec is selected; a spec carries only the starting crop geometry and which rules apply.

**Estimates are marked as estimates.** Chin and crown carry `estimated: true`, the UI shows an `AUTO` badge, and the user can drag either. A value the regulation does not state numerically (the Croatian chin line exists only as a graphic on the MUP template) stays `null` rather than becoming an invented number.

**Untrusted numbers do not pose as verified.** `confidence` distinguishes `verified` (a named regulation with a source), `baseline` (widely used, without a single authoritative source) and `community`. A test in `spec.rs` ensures no non-Croatian spec claims to be `verified`.

**What cannot be measured does not pass — it is reported as not checked.**

**Offline is tested, not promised.** The CSP (`connect-src 'none'`) constrains the webview only. The Rust side is checked separately: a test walks the dependency tree and fails if any HTTP client (`reqwest`, `hyper`, `ureq`, …) is compiled in at all. This is why the `ort` `download-binaries` feature is off and ONNX Runtime ships with the installer instead.

**The mask is not an edited bitmap.** The model's automatic output and the user's strokes are kept apart, exactly as detection and manual overrides are. Re-running the model does not throw away hand work, and undo costs nothing but dropping the last stroke.

**One config file, one atomic write.** Calibrations, presets and sheet settings share `config.toml` and go through the same type — so saving a preset cannot drop the calibrations, the classic bug when two features own the same file.

---

## Document formats (specs)

### Croatian — `specs/hr.json`

| ID | Format | Notes |
|----|--------|-------|
| `hr-passport-35x45` | 35 × 45 mm | Croatian documents. Head height: adults 31.5–36 mm, children from 22.5 mm |
| `hr-putni-list-30x35` | 30 × 35 mm | Emergency travel document. Inherits the rules, overrides size and head height |
| `hr-intl-treaty-35x45` | 35 × 45 mm | Travel document under international treaty |

### International — `specs/international.json`

| ID | Format | Notes |
|----|--------|-------|
| `us-visa-51x51` | 51 × 51 mm | US visa and passport (2×2") |
| `schengen-visa-35x45` | 35 × 45 mm | Schengen visa |
| `generic-25x30` | 25 × 30 mm | Small format, no checks |
| `generic-30x40` | 30 × 40 mm | Small format, no checks |
| `generic-40x60` | 40 × 60 mm | No checks |
| `free-custom` | user-entered | Free dimensions, 20–200 mm |

The files are separate deliberately: inheritance resolves *within* a file, so an international spec cannot inherit from a Croatian one. The two sets have different provenance and must not silently share numbers.

### Spec structure

```json
{
  "id": "hr-passport-35x45",
  "print":    { "width_mm": 35, "height_mm": 45, "min_dpi": 300 },
  "geometry": {
    "anchor": "chin_line",
    "head_measure": "chin_to_crown",
    "age_variants": [
      { "id": "adult", "age_min_years": 12, "head_height_mm": { "min": 31.5, "max": 36 } }
    ],
    "chin_line_from_bottom_mm": { "value": null }
  },
  "pose":       { "max_roll_deg": 5 },
  "background": { "allowed": [{ "name": "light_grey", "rgb": [235,235,235], "default": true }] },
  "rules":      [{ "id": "sharpness", "severity": "warning", "min_laplacian_var": 100 }],
  "confidence": "verified"
}
```

`geometry: null` means **a free crop with no guides**. An empty `rules` means no checks at all.

Specs are compiled into the binary (`include_str!`) — the application cannot be broken by a missing file, and the offline guarantee stays trivial.

---

## IPC surface

All commands registered in [src-tauri/src/lib.rs](src-tauri/src/lib.rs):

| Area | Commands |
|------|----------|
| **Printers** | `list_printers`, `printer_capabilities` |
| **Layout** | `solve_layout`, `solve_mixed_layout` |
| **Printing** | `print_sheet`, `print_mixed_sheet`, `print_calibration_square` |
| **Calibration** | `get_calibration`, `save_calibration` |
| **Face** | `detect_face`, `compute_crop` |
| **Background** | `segment_background`, `add_mask_stroke`, `undo_mask_stroke`, `reset_mask_edits`, `set_mask_threshold`, `clear_mask`, `background_uniformity` |
| **Specs** | `list_specs`, `validate_photo`, `analyse_image` |
| **Settings** | `get_sheet_settings`, `save_sheet_settings`, `list_presets`, `save_preset`, `load_preset`, `delete_preset` |

Errors come back as `UiError { key, params }` — an i18n key with parameters, never a finished sentence.

---

## Settings and configuration

All persistent state lives in one TOML file: `%APPDATA%\...\config.toml` (`~/.config` elsewhere). Deliberately **not next to the executable**, which may sit in `Program Files` where a normal user cannot write. Deliberately **not localStorage** — a preset the user spent time building is not something that should vanish when browser storage is cleared.

It holds three tables:

- **`[calibrations]`** — corrections keyed by `(printer, paper, borderless)`
- **`[presets]`** — named presets (bounded name length, parameters only)
- **`[sheet]`** — sheet settings

Sheet setting defaults:

| Setting | Default |
|---------|---------|
| `paper_id` | `10x15` |
| `count` | 6 |
| `margin_mm` | 3.0 |
| `gutter_mm` | 2.0 |
| `cut_marks` | on |
| `quarter_turn`, `turn_photo`, `align_top_left` | off |
| `font_scale_percent` | 100 (range 50–200, clamped on save) |
| `theme` | `light` |

Every field is `#[serde(default)]`, so a preset written by an older build still loads after a field is added — rather than failing the whole file and taking the calibrations down with it.

---

## Development and running

### Prerequisites

- **Rust** 1.77+ ([rustup.rs](https://rustup.rs))
- **Node.js** 18+
- **Windows** (printing is Win32; everything else compiles anywhere)
- Visual Studio Build Tools with the C++ workload

### First-time setup

```powershell
npm install
.\fetch-models.ps1     # fetches the ONNX Runtime DLL (14 MB) and the segmentation model
```

`fetch-models.ps1` is a **development step** — it verifies SHA256 and skips files already downloaded. The built application never downloads anything.

The models are in git (YuNet 227 KB, U²-Netp 4.4 MB) because pinning their exact contents is worth more than the space saved. `onnxruntime.dll` is not, because it is 14 MB and identical for everyone.

### Running

```powershell
.\run.ps1
```

The script works around two Windows-specific problems: `cargo` not being on `PATH` in a shell started before Rust was installed, and Smart App Control blocking freshly linked unsigned binaries for a while. It also starts Vite if it is not already serving and stops a previous instance of the app.

Manually:

```powershell
npm run dev            # Vite on :5173
cargo build -p documents-app
npm run tauri build    # NSIS installer
```

### Packaging

Packaged as an **NSIS current-user installer**, with the models and ONNX Runtime as resources beside the binary. The application checks both locations (repository root in development, next to the binary when installed), so it runs either way.

---

## Testing

Roughly **250 tests**, concentrated in the `domain` crate.

```powershell
cargo test --workspace
```

Kinds of tests:

- **Unit tests**, beside the code they cover (`geometry`, `layout`, `rules`, `spec`, `render`, `mask`, `adjust`, `resample`, `units`, `decode`, `calibration`, `presets`)
- **Property tests** (`layout_properties.rs`, proptest) — layout invariants over random inputs, with pinned regressions
- **Synthetic face** (`synthetic_face.rs`) — the geometry chain without a real photograph
- **Calibration geometry** (`calibration_geometry.rs`)
- **Command contract** (`commands_contract.rs`) — the shape of the IPC surface
- **Offline guarantee** (`offline_guarantee.rs`) — fails if an HTTP client creeps into the dependency tree

A substantial share of the tests guard **real regressions**, with numbers from actual photographs and print runs in the comments: a crown estimate that landed outside the image, an integer overflow in the request-length check, grey guides that printed as nothing at all on a photo printer.

---

## Repository structure

```
Documents_app/
├── crates/
│   ├── domain/          pure logic: geometry, layout, rules, rendering
│   ├── vision/          ONNX: YuNet detection, U²-Netp segmentation
│   └── platform/        printers (Win32 GDI), calibration, configuration
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs       command registration, model preloading
│   │   ├── commands.rs  printers, layout, printing, calibration
│   │   ├── face.rs      face detection, crop computation
│   │   ├── background.rs segmentation and mask editing
│   │   ├── spec.rs      spec listing, validation
│   │   └── presets.rs   presets and settings
│   └── tauri.conf.json  window, CSP, bundling
├── src/
│   ├── App.tsx          wizard state
│   ├── components/      DropZone, FormatPicker, PhotoCanvas, SheetPreview, …
│   └── lib/             ipc.ts, i18n.ts, history.ts, units.ts
├── specs/
│   ├── hr.json          Croatian specs
│   └── international.json
├── models/              YuNet + U²-Netp (in git)
├── runtime/             onnxruntime.dll (fetched by script)
├── fetch-models.ps1
└── run.ps1
```

---

## Privacy

Document photographs are biometric data. The application is built so that it cannot send them anywhere:

- the webview has `connect-src 'none'`
- no HTTP client exists in the Rust binary, and a test fails if one appears
- models ship with the application rather than being downloaded
- nothing is transmitted, not even for error reporting
- `test-data/` is in `.gitignore` — test photographs never reach the repository

---

## License

MIT. The models carry their own licenses — see [models/LICENSE-yunet.txt](models/LICENSE-yunet.txt) and [models/README.md](models/README.md).
