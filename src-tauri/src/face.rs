//! Face detection commands.
//!
//! The detector is created once and reused: loading the model costs about a
//! second, which would be paid on every call otherwise.

use domain::geometry::{
    estimate_head_anchors, solve_crop, CropError, CropTarget, Detection, Rect,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use vision::decode::primary_face;
use vision::detect::FaceDetector;

use crate::commands::UiError;

/// Lazily created detector, shared across calls.
#[derive(Default)]
pub struct DetectorState(pub Mutex<Option<FaceDetector>>);

/// Build the detector ahead of time, so loading a photo does not wait for it.
///
/// Same reasoning as `background::preload`: the session build is a fixed cost
/// that has no reason to land on the user's first action. Silent on failure —
/// `detect_face` still loads the model itself if this did not run.
pub fn preload(state: &DetectorState) {
    let Ok(mut guard) = state.0.lock() else { return };
    if guard.is_some() {
        return;
    }
    ensure_runtime_path();
    let Some(model) = resource_path("models/face_detection_yunet_2023mar.onnx") else {
        return;
    };
    if let Ok(detector) = FaceDetector::from_path(&model) {
        *guard = Some(detector);
    }
}

/// Where the bundled files live relative to the executable.
///
/// In development they sit in the repository root; in an installed build they
/// are next to the binary. Both are checked so the app runs either way.
///
/// The bundler also has a say. The paths in `tauri.conf.json` start with `../`,
/// because the models sit in the repository root rather than under `src-tauri`,
/// and a resource whose source escapes the config directory is placed under
/// `_up_/` in the installed layout. That is why `_up_` is searched too: without
/// it an installed build finds nothing, while a development build — where the
/// working directory happens to contain `models/` — works, so the difference
/// only ever shows up after installing.
pub(crate) fn resource_path(relative: &str) -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(relative));
            candidates.push(dir.join("_up_").join(relative));
            // Tauri v1 laid resources out under `resources/`.
            candidates.push(dir.join("resources").join(relative));
            candidates.push(dir.join("resources").join("_up_").join(relative));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(relative));
        // Running from src-tauri during development.
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join(relative));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Point the ONNX Runtime loader at the bundled library.
///
/// `ort` is built with `load-dynamic`, so it needs an explicit path rather
/// than linking the runtime in. Nothing is downloaded; the DLL ships with the
/// app.
pub(crate) fn ensure_runtime_path() {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let name = if cfg!(windows) {
        "runtime/onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "runtime/libonnxruntime.dylib"
    } else {
        "runtime/libonnxruntime.so"
    };
    if let Some(p) = resource_path(name) {
        // SAFETY: called under the detector mutex before any ort use, so no
        // other thread can be reading the environment concurrently.
        unsafe { std::env::set_var("ORT_DYLIB_PATH", p) };
    }
}

#[derive(Debug, Serialize)]
pub struct PointDto {
    pub x: f64,
    pub y: f64,
}

impl From<domain::geometry::Point> for PointDto {
    fn from(p: domain::geometry::Point) -> Self {
        Self { x: p.x, y: p.y }
    }
}

#[derive(Debug, Serialize)]
pub struct RectDto {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl From<Rect> for RectDto {
    fn from(r: Rect) -> Self {
        Self { x: r.x, y: r.y, width: r.width, height: r.height }
    }
}

#[derive(Debug, Serialize)]
pub struct DetectionDto {
    pub face_box: RectDto,
    pub right_eye: PointDto,
    pub left_eye: PointDto,
    pub nose: PointDto,
    pub chin: PointDto,
    pub crown: PointDto,
    pub confidence: f32,
    pub roll_deg: f64,
    pub eye_distance_px: f64,
    /// True while chin and crown come from the estimator rather than the user.
    pub anchors_estimated: bool,
}

fn to_dto(d: &Detection) -> DetectionDto {
    let a = estimate_head_anchors(d);
    DetectionDto {
        face_box: d.bbox.into(),
        right_eye: d.landmarks.right_eye.into(),
        left_eye: d.landmarks.left_eye.into(),
        nose: d.landmarks.nose.into(),
        chin: a.chin.into(),
        crown: a.crown.into(),
        confidence: d.confidence,
        roll_deg: d.landmarks.roll_degrees(),
        eye_distance_px: d.landmarks.eye_distance(),
        anchors_estimated: a.estimated,
    }
}

/// Detect the subject's face in raw RGBA pixels from a canvas.
///
/// Pixels rather than an encoded file because the webview already decoded the
/// image, and re-encoding it only to decode it again in Rust would be wasteful
/// and would pull a JPEG decoder into the build.
#[tauri::command]
pub fn detect_face(
    state: tauri::State<'_, DetectorState>,
    request: tauri::ipc::Request<'_>,
) -> Result<Option<DetectionDto>, UiError> {
    let width = crate::commands::header_u32(&request, "x-width")?;
    let height = crate::commands::header_u32(&request, "x-height")?;
    let rgba = crate::commands::raw_body(&request)?;

    if width == 0 || height == 0 {
        return Err(UiError::new("error.image.empty"));
    }
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(UiError::with(
            "error.image.size_mismatch",
            serde_json::json!({ "expected": expected, "got": rgba.len() }),
        ));
    }

    // Drop alpha: the detector works on RGB.
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }
    let img = image::RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| UiError::new("error.image.decode_failed"))?;

    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.is_none() {
        ensure_runtime_path();
        let model = resource_path("models/face_detection_yunet_2023mar.onnx")
            .ok_or_else(|| UiError::new("error.model.not_found"))?;
        let detector = FaceDetector::from_path(&model).map_err(|e| {
            UiError::with("error.model.load_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;
        *guard = Some(detector);
    }
    let detector = guard.as_mut().expect("detector was just created");

    let faces = detector.detect(&img).map_err(|e| {
        UiError::with("error.detect.failed", serde_json::json!({ "detail": e.to_string() }))
    })?;

    Ok(primary_face(&faces).map(to_dto))
}

#[derive(Debug, Deserialize)]
pub struct CropRequest {
    pub chin_x: f64,
    pub chin_y: f64,
    pub crown_x: f64,
    pub crown_y: f64,
    pub image_width: f64,
    pub image_height: f64,
    pub photo_width_mm: f64,
    pub photo_height_mm: f64,
    pub head_height_mm: f64,
    pub chin_from_bottom_mm: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct CropDto {
    pub rect: RectDto,
    /// Highest DPI this crop supports without upscaling.
    pub max_lossless_dpi: f64,
}

/// Compute the crop for the given anchors.
///
/// Takes anchors as plain numbers rather than a detection, so the UI can pass
/// user-dragged positions exactly as it passes automatic ones.
#[tauri::command]
pub fn compute_crop(req: CropRequest) -> Result<CropDto, UiError> {
    use domain::geometry::{HeadAnchors, Point};

    let anchors = HeadAnchors {
        chin: Point::new(req.chin_x, req.chin_y),
        crown: Point::new(req.crown_x, req.crown_y),
        estimated: false,
    };
    let target = CropTarget {
        photo_width_mm: req.photo_width_mm,
        photo_height_mm: req.photo_height_mm,
        head_height_mm: req.head_height_mm,
        chin_from_bottom_mm: req.chin_from_bottom_mm,
    };
    let image = Rect::new(0.0, 0.0, req.image_width, req.image_height);

    match solve_crop(&anchors, &target, &image) {
        Ok(rect) => {
            let dpi_w = rect.width * 25.4 / req.photo_width_mm;
            let dpi_h = rect.height * 25.4 / req.photo_height_mm;
            Ok(CropDto { rect: rect.into(), max_lossless_dpi: dpi_w.min(dpi_h) })
        }
        Err(CropError::CropOutsideImage {
            overflow_left_px,
            overflow_top_px,
            overflow_right_px,
            overflow_bottom_px,
            min_head_height_mm,
        }) => Err(UiError::with(
            "error.crop.outside_image",
            serde_json::json!({
                "left": overflow_left_px.round(),
                "top": overflow_top_px.round(),
                "right": overflow_right_px.round(),
                "bottom": overflow_bottom_px.round(),
                "minHeadMm": (min_head_height_mm * 10.0).round() / 10.0,
            }),
        )),
        Err(CropError::DegenerateHead) => Err(UiError::new("error.crop.degenerate_head")),
        Err(CropError::InvalidTarget) => Err(UiError::new("error.layout.invalid_dimensions")),
    }
}
