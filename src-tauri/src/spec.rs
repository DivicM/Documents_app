//! Spec listing and photo validation commands.

use domain::geometry::{Point, Rect};
use domain::rules::{validate as run_rules, Measurements, RuleResult};
use domain::spec::{builtin_specs, Spec};
use serde::{Deserialize, Serialize};

use crate::commands::UiError;

/// A spec as the format picker needs it.
#[derive(Debug, Serialize)]
pub struct SpecSummary {
    pub id: String,
    pub name: String,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Head height to start from: the middle of the permitted range.
    pub head_height_mm: Option<f64>,
    pub min_dpi: f64,
    pub default_count: u32,
    pub cut_marks: bool,
    pub background_rgb: Option<[u8; 3]>,
    /// True when the spec imposes no geometry, i.e. free crop.
    pub free_mode: bool,
    /// "verified", "baseline" or "community". The picker warns on anything but
    /// verified, so the user knows which numbers to check themselves.
    pub confidence: String,
    /// Country code, for grouping the list.
    pub country: Option<String>,
}

fn summarise(spec: &Spec, lang: &str) -> SpecSummary {
    let head_height_mm = spec
        .geometry
        .as_ref()
        .and_then(|g| g.variant_for_age(None))
        .map(|v| (v.head_height_mm.midpoint() * 100.0).round() / 100.0);

    SpecSummary {
        id: spec.id.clone(),
        name: spec.name(lang),
        width_mm: spec.print.width_mm,
        height_mm: spec.print.height_mm,
        head_height_mm,
        min_dpi: spec.print.min_dpi,
        default_count: spec.layout_defaults.count,
        cut_marks: spec.layout_defaults.cut_marks,
        background_rgb: spec.background.as_ref().map(|b| b.default_rgb()),
        free_mode: spec.is_free_mode(),
        confidence: match spec.confidence {
            domain::spec::Confidence::Verified => "verified",
            domain::spec::Confidence::Baseline => "baseline",
            domain::spec::Confidence::Community => "community",
        }
        .to_string(),
        country: spec.country.clone(),
    }
}

/// Every bundled spec, for the format picker.
#[tauri::command]
pub fn list_specs(lang: Option<String>) -> Result<Vec<SpecSummary>, UiError> {
    let lang = lang.unwrap_or_else(|| "hr".to_string());
    let specs = builtin_specs().map_err(|e| {
        UiError::with("error.spec.load_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;
    Ok(specs.iter().map(|s| summarise(s, &lang)).collect())
}

/// Measurements the frontend has already gathered.
#[derive(Debug, Deserialize)]
pub struct ValidateRequest {
    pub spec_id: String,
    pub head_px: f64,
    pub crop_x: f64,
    pub crop_y: f64,
    pub crop_width: f64,
    pub crop_height: f64,
    pub head_centre_x: f64,
    pub head_centre_y: f64,
    pub eye_distance_px: f64,
    pub roll_deg: f64,
    pub dpi: f64,
    #[serde(default)]
    pub age_years: Option<f64>,
    #[serde(default)]
    pub background_stddev: Option<f64>,
    #[serde(default)]
    pub sharpness: Option<f64>,
    #[serde(default)]
    pub clipped_shadows: Option<f64>,
    #[serde(default)]
    pub clipped_highlights: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ValidationDto {
    pub results: Vec<RuleResult>,
    /// True when something failed at error severity.
    pub blocking: bool,
}

/// Validate a photo against a spec.
#[tauri::command]
pub fn validate_photo(req: ValidateRequest) -> Result<ValidationDto, UiError> {
    let specs = builtin_specs().map_err(|e| {
        UiError::with("error.spec.load_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;
    let spec = specs
        .iter()
        .find(|s| s.id == req.spec_id)
        .ok_or_else(|| {
            UiError::with("error.spec.not_found", serde_json::json!({ "id": req.spec_id }))
        })?;

    let m = Measurements {
        head_px: req.head_px,
        crop: Rect::new(req.crop_x, req.crop_y, req.crop_width, req.crop_height),
        head_centre: Point::new(req.head_centre_x, req.head_centre_y),
        eye_distance_px: req.eye_distance_px,
        roll_deg: req.roll_deg,
        background_stddev: req.background_stddev,
        sharpness: req.sharpness,
        clipped_shadows: req.clipped_shadows,
        clipped_highlights: req.clipped_highlights,
    };

    let results = run_rules(spec, &m, req.age_years, req.dpi);
    let blocking = domain::rules::has_blocking_failure(&results);
    Ok(ValidationDto { results, blocking })
}

/// Exposure statistics for a photo, feeding the sharpness and exposure rules.
///
/// Computed in Rust rather than the webview because the same code must produce
/// the numbers the validator sees and the numbers the print path uses.
#[tauri::command]
pub fn analyse_image(request: tauri::ipc::Request<'_>) -> Result<ImageStatsDto, UiError> {
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

    // The face rectangle is optional; absent headers mean "measure everywhere".
    let face = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok())
    };

    let (shadows, highlights) = domain::adjust::clipping(rgba);
    // Sharpness over the face, so a busy background does not mask a soft face.
    let region = match (face("x-face-x"), face("x-face-y"), face("x-face-w"), face("x-face-h")) {
        (Some(x), Some(y), Some(w), Some(h)) => Some((x, y, w, h)),
        _ => None,
    };
    let sharpness = domain::adjust::laplacian_variance(rgba, width, height, region);

    Ok(ImageStatsDto { clipped_shadows: shadows, clipped_highlights: highlights, sharpness })
}

#[derive(Debug, Serialize)]
pub struct ImageStatsDto {
    pub clipped_shadows: f64,
    pub clipped_highlights: f64,
    pub sharpness: Option<f64>,
}
