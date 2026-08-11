//! Background segmentation and mask editing commands.
//!
//! The mask lives here rather than in the frontend: it is up to a megabyte, and
//! shipping it across the IPC boundary on every brush stroke would make editing
//! sluggish. The UI sends strokes and receives a small preview.

use domain::mask::{apply_edits, apply_threshold, AlphaMask, BrushMode, BrushStroke, MaskEdits};
use serde::Deserialize;
use std::sync::Mutex;
use vision::segment::BackgroundSegmenter;

use crate::commands::UiError;
use crate::face::{ensure_runtime_path, resource_path};

/// The segmenter plus the mask it last produced, and the user's edits.
#[derive(Default)]
pub struct SegmentState(pub Mutex<SegmentInner>);

/// Build the segmenter and keep it, so the first click does not pay for it.
///
/// Loading the model and building the ONNX session costs about 1.6s, which was
/// previously charged to whoever first pressed "remove background". Called on a
/// background thread at startup; failure is silent because the command loads
/// the model itself if this has not finished or did not work.
pub fn preload(state: &SegmentState) {
    let Ok(mut guard) = state.0.lock() else { return };
    if guard.segmenter.is_some() {
        return;
    }
    ensure_runtime_path();
    let Some(model) = resource_path("models/u2netp.onnx") else { return };
    if let Ok(seg) = BackgroundSegmenter::from_path(&model) {
        guard.segmenter = Some(seg);
    }
}

pub struct SegmentInner {
    segmenter: Option<BackgroundSegmenter>,
    /// The model's output, never modified by editing.
    auto_mask: Option<AlphaMask>,
    edits: MaskEdits,
    /// Edge tightness; 128 leaves the model's own decision alone.
    threshold: u8,
}

impl Default for SegmentInner {
    fn default() -> Self {
        // Not derived: a default of 0 would drive every mask to full coverage.
        Self { segmenter: None, auto_mask: None, edits: MaskEdits::default(), threshold: 128 }
    }
}

/// Bytes of the header that precedes the mask in a raw response.
///
/// Layout: width, height, subject ratio (all little-endian u32/f32), then the
/// backend name length, then the name, then the mask itself. Sending the mask
/// as a JSON number array instead would inflate 100KB to roughly 300KB of text
/// and cost more to parse than the segmentation takes to run.
const MASK_HEADER_LEN: usize = 4 + 4 + 4 + 4;

fn mask_response(mask: &AlphaMask, backend: &str) -> tauri::ipc::Response {
    let subject = mask.data.iter().filter(|&&v| v > 128).count();
    let ratio = subject as f32 / mask.data.len().max(1) as f32;
    let name = backend.as_bytes();

    let mut out = Vec::with_capacity(MASK_HEADER_LEN + name.len() + mask.data.len());
    out.extend_from_slice(&mask.width.to_le_bytes());
    out.extend_from_slice(&mask.height.to_le_bytes());
    out.extend_from_slice(&ratio.to_le_bytes());
    out.extend_from_slice(&(name.len() as u32).to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(&mask.data);
    tauri::ipc::Response::new(out)
}

/// Run segmentation on raw RGBA pixels and return the resulting mask.
///
/// Pixels arrive as a raw body rather than a JSON number array. A 2000x1333
/// photo is 10.7MB, which as JSON becomes 32MB of text costing roughly two
/// seconds to encode and parse — far more than the 85ms the model itself takes.
#[tauri::command]
pub fn segment_background(
    state: tauri::State<'_, SegmentState>,
    request: tauri::ipc::Request<'_>,
) -> Result<tauri::ipc::Response, UiError> {
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

    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;

    if guard.segmenter.is_none() {
        ensure_runtime_path();
        let model = resource_path("models/u2netp.onnx")
            .ok_or_else(|| UiError::new("error.model.not_found"))?;
        let seg = BackgroundSegmenter::from_path(&model).map_err(|e| {
            UiError::with("error.model.load_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;
        guard.segmenter = Some(seg);
    }

    let segmenter = guard.segmenter.as_mut().expect("segmenter was just created");
    let backend = format!("{:?}", segmenter.backend());
    // Straight from the RGBA the webview sent, with no intermediate image.
    let mask = segmenter.segment_rgba(rgba, width, height).map_err(|e| {
        UiError::with("error.segment.failed", serde_json::json!({ "detail": e.to_string() }))
    })?;

    // A new automatic mask replaces the old one; strokes are deliberately kept,
    // matching how re-detection preserves overrides.
    guard.auto_mask = Some(mask);
    let effective = current_mask(&guard).expect("auto mask was just set");
    Ok(mask_response(&effective, &backend))
}

/// The mask actually in use: automatic output, thresholded, then edited.
///
/// Order matters. The threshold adjusts the model's edge, and brush strokes go
/// on top so a deliberate correction is never undone by a slider.
fn current_mask(inner: &SegmentInner) -> Option<AlphaMask> {
    let auto = inner.auto_mask.as_ref()?;
    let adjusted = apply_threshold(auto, inner.threshold);
    Some(apply_edits(&adjusted, &inner.edits))
}

/// Set the edge threshold and return the updated mask.
#[tauri::command]
pub fn set_mask_threshold(
    state: tauri::State<'_, SegmentState>,
    threshold: u8,
) -> Result<tauri::ipc::Response, UiError> {
    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.auto_mask.is_none() {
        return Err(UiError::new("error.mask.not_segmented"));
    }
    guard.threshold = threshold;
    let effective = current_mask(&guard).expect("auto mask checked above");
    Ok(mask_response(&effective, "edited"))
}

#[derive(Debug, Deserialize)]
pub struct StrokeRequest {
    /// "keep" paints subject back in, "erase" removes it.
    pub mode: String,
    pub radius: f32,
    pub feather: f32,
    /// Points in mask coordinates.
    pub points: Vec<(f32, f32)>,
}

/// Add one brush stroke and return the updated mask.
#[tauri::command]
pub fn add_mask_stroke(
    state: tauri::State<'_, SegmentState>,
    stroke: StrokeRequest,
) -> Result<tauri::ipc::Response, UiError> {
    let mode = match stroke.mode.as_str() {
        "keep" => BrushMode::Keep,
        "erase" => BrushMode::Erase,
        _ => return Err(UiError::new("error.mask.unknown_mode")),
    };
    if stroke.points.is_empty() {
        return Err(UiError::new("error.mask.empty_stroke"));
    }

    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.auto_mask.is_none() {
        return Err(UiError::new("error.mask.not_segmented"));
    }

    let mut s = BrushStroke::new(mode, stroke.radius, stroke.feather);
    for (x, y) in stroke.points {
        s.push(x, y);
    }
    guard.edits.push(s);

    let effective = current_mask(&guard).expect("auto mask checked above");
    Ok(mask_response(&effective, "edited"))
}

/// Undo the most recent stroke.
#[tauri::command]
pub fn undo_mask_stroke(state: tauri::State<'_, SegmentState>) -> Result<tauri::ipc::Response, UiError> {
    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.auto_mask.is_none() {
        return Err(UiError::new("error.mask.not_segmented"));
    }
    guard.edits.undo();
    let effective = current_mask(&guard).expect("auto mask checked above");
    Ok(mask_response(&effective, "edited"))
}

/// Discard every stroke, returning to the model's own output.
#[tauri::command]
pub fn reset_mask_edits(state: tauri::State<'_, SegmentState>) -> Result<tauri::ipc::Response, UiError> {
    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.auto_mask.is_none() {
        return Err(UiError::new("error.mask.not_segmented"));
    }
    guard.edits.clear();
    let effective = current_mask(&guard).expect("auto mask checked above");
    Ok(mask_response(&effective, "auto"))
}

/// Forget the mask, so a newly loaded photo does not inherit the previous one.
#[tauri::command]
pub fn clear_mask(state: tauri::State<'_, SegmentState>) -> Result<(), UiError> {
    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    guard.auto_mask = None;
    guard.edits.clear();
    guard.threshold = 128;
    Ok(())
}

/// Standard deviation of the background, for the uniformity rule.
///
/// Measured only where the mask says background, so the subject's own colours
/// do not count as unevenness. Returns `None` when no mask exists or the
/// background is too small a sample to mean anything.
#[tauri::command]
pub fn background_uniformity(
    state: tauri::State<'_, SegmentState>,
    request: tauri::ipc::Request<'_>,
) -> Result<Option<f64>, UiError> {
    let width = crate::commands::header_u32(&request, "x-width")?;
    let height = crate::commands::header_u32(&request, "x-height")?;
    let rgba = crate::commands::raw_body(&request)?;

    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(UiError::with(
            "error.image.size_mismatch",
            serde_json::json!({ "expected": expected, "got": rgba.len() }),
        ));
    }

    let guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    let Some(mask) = current_mask(&guard) else {
        return Ok(None);
    };

    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut n = 0u64;

    for y in 0..height {
        let my = (y as u64 * mask.height as u64 / height.max(1) as u64) as u32;
        for x in 0..width {
            let mx = (x as u64 * mask.width as u64 / width.max(1) as u64) as u32;
            // Well inside the background, clear of the soft edge where subject
            // and background blend and the variance is meaningless.
            if mask.at(mx, my) > 32 {
                continue;
            }
            let i = (y as usize * width as usize + x as usize) * 4;
            let luma = 0.299 * rgba[i] as f64 + 0.587 * rgba[i + 1] as f64 + 0.114 * rgba[i + 2] as f64;
            sum += luma;
            sum_sq += luma * luma;
            n += 1;
        }
    }

    // Too few background pixels to say anything useful.
    if n < 100 {
        return Ok(None);
    }
    let mean = sum / n as f64;
    let variance = (sum_sq / n as f64 - mean * mean).max(0.0);
    Ok(Some(variance.sqrt()))
}
