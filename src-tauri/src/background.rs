//! Background segmentation and mask editing commands.
//!
//! The mask lives here rather than in the frontend: it is up to a megabyte, and
//! shipping it across the IPC boundary on every brush stroke would make editing
//! sluggish. The UI sends strokes and receives a small preview.

use domain::mask::{apply_edits, apply_threshold, AlphaMask, BrushMode, BrushStroke, MaskEdits};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use vision::segment::{BackgroundSegmenter, INPUT_SIZE};

use crate::commands::UiError;
use crate::face::{ensure_runtime_path, resource_path};

/// The segmenter plus the mask it last produced, and the user's edits.
#[derive(Default)]
pub struct SegmentState(pub Mutex<SegmentInner>);

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

#[derive(Debug, Serialize)]
pub struct MaskDto {
    pub width: u32,
    pub height: u32,
    /// Single-channel coverage, one byte per pixel.
    pub data: Vec<u8>,
    /// Which execution provider ran the model, so the UI can explain a slow run.
    pub backend: String,
    /// Fraction of the mask that is subject, for a quick sanity display.
    pub subject_ratio: f32,
}

fn to_dto(mask: &AlphaMask, backend: &str) -> MaskDto {
    let subject = mask.data.iter().filter(|&&v| v > 128).count();
    MaskDto {
        width: mask.width,
        height: mask.height,
        data: mask.data.clone(),
        backend: backend.to_string(),
        subject_ratio: subject as f32 / mask.data.len().max(1) as f32,
    }
}

/// Run segmentation on raw RGBA pixels and return the resulting mask.
#[tauri::command]
pub fn segment_background(
    state: tauri::State<'_, SegmentState>,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<MaskDto, UiError> {
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

    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for px in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&px[..3]);
    }
    let img = image::RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| UiError::new("error.image.decode_failed"))?;

    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;

    if guard.segmenter.is_none() {
        ensure_runtime_path();
        let model = resource_path("models/birefnet_lite_fp16.onnx")
            .ok_or_else(|| UiError::new("error.model.not_found"))?;
        let seg = BackgroundSegmenter::from_path(&model).map_err(|e| {
            UiError::with("error.model.load_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;
        guard.segmenter = Some(seg);
    }

    let segmenter = guard.segmenter.as_mut().expect("segmenter was just created");
    let backend = format!("{:?}", segmenter.backend());
    let mask = segmenter.segment(&img).map_err(|e| {
        UiError::with("error.segment.failed", serde_json::json!({ "detail": e.to_string() }))
    })?;

    // A new automatic mask replaces the old one; strokes are deliberately kept,
    // matching how re-detection preserves overrides.
    guard.auto_mask = Some(mask);
    let effective = current_mask(&guard).expect("auto mask was just set");
    Ok(to_dto(&effective, &backend))
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
) -> Result<MaskDto, UiError> {
    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.auto_mask.is_none() {
        return Err(UiError::new("error.mask.not_segmented"));
    }
    guard.threshold = threshold;
    let effective = current_mask(&guard).expect("auto mask checked above");
    Ok(to_dto(&effective, "edited"))
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
) -> Result<MaskDto, UiError> {
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
    Ok(to_dto(&effective, "edited"))
}

/// Undo the most recent stroke.
#[tauri::command]
pub fn undo_mask_stroke(state: tauri::State<'_, SegmentState>) -> Result<MaskDto, UiError> {
    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.auto_mask.is_none() {
        return Err(UiError::new("error.mask.not_segmented"));
    }
    guard.edits.undo();
    let effective = current_mask(&guard).expect("auto mask checked above");
    Ok(to_dto(&effective, "edited"))
}

/// Discard every stroke, returning to the model's own output.
#[tauri::command]
pub fn reset_mask_edits(state: tauri::State<'_, SegmentState>) -> Result<MaskDto, UiError> {
    let mut guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    if guard.auto_mask.is_none() {
        return Err(UiError::new("error.mask.not_segmented"));
    }
    guard.edits.clear();
    let effective = current_mask(&guard).expect("auto mask checked above");
    Ok(to_dto(&effective, "auto"))
}

/// How many strokes are currently applied, for enabling the undo button.
#[tauri::command]
pub fn mask_stroke_count(state: tauri::State<'_, SegmentState>) -> Result<usize, UiError> {
    let guard = state.0.lock().map_err(|_| UiError::new("error.internal.lock"))?;
    Ok(guard.edits.strokes.len())
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

/// The mask resolution, which is fixed by the model.
#[tauri::command]
pub fn mask_size() -> u32 {
    INPUT_SIZE
}

/// Standard deviation of the background, for the uniformity rule.
///
/// Measured only where the mask says background, so the subject's own colours
/// do not count as unevenness. Returns `None` when no mask exists or the
/// background is too small a sample to mean anything.
#[tauri::command]
pub fn background_uniformity(
    state: tauri::State<'_, SegmentState>,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<Option<f64>, UiError> {
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
