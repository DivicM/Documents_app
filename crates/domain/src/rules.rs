//! Validating a photo against a spec.
//!
//! Pure functions over measurements that have already been taken: no image, no
//! model, no UI. Messages are i18n keys plus parameters, never prose, so the
//! frontend decides the wording (§8).
//!
//! Rules the detector cannot measure are reported as [`Status::NotChecked`]
//! rather than passing. The brief forbids promising a guarantee the program
//! cannot give, and a green tick beside "mouth closed" would be exactly that.

use crate::geometry::{Point, Rect};
use crate::spec::{Severity, Spec};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    /// The criterion exists in the spec but nothing here can measure it.
    NotChecked,
}

/// A concrete change that would make a failing rule pass.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum FixHint {
    /// Set the head height to this many millimetres.
    SetHeadHeightMm(f64),
    /// Rotate by this many degrees to level the eyes.
    SetRotationDeg(f64),
    /// Reduce the DPI to this to avoid upscaling.
    SetDpi(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleResult {
    pub rule_id: String,
    pub status: Status,
    pub severity: Severity,
    /// i18n key, resolved by the frontend.
    pub message_key: String,
    pub params: serde_json::Value,
    pub fix_hint: Option<FixHint>,
}

impl RuleResult {
    fn new(id: &str, severity: Severity, status: Status, key: &str) -> Self {
        Self {
            rule_id: id.to_string(),
            status,
            severity,
            message_key: key.to_string(),
            params: serde_json::json!({}),
            fix_hint: None,
        }
    }

    fn with_params(mut self, params: serde_json::Value) -> Self {
        self.params = params;
        self
    }

    fn with_fix(mut self, fix: FixHint) -> Self {
        self.fix_hint = Some(fix);
        self
    }
}

/// Everything measurable about a photo, gathered before validation.
///
/// Assembled by the caller from the detection, the crop and the mask, so this
/// module needs no knowledge of where any of it came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurements {
    /// Head height in source pixels (chin to crown).
    pub head_px: f64,
    /// The crop actually in use, in source pixels.
    pub crop: Rect,
    /// Centre of the head, for the centring check.
    pub head_centre: Point,
    /// Pupil-to-pupil distance in source pixels.
    pub eye_distance_px: f64,
    /// Head tilt in degrees, positive clockwise.
    pub roll_deg: f64,
    /// Standard deviation of the background, if a mask was computed.
    pub background_stddev: Option<f64>,
    /// Variance of the Laplacian over the face, if computed.
    pub sharpness: Option<f64>,
    /// Fraction of pixels clipped to pure black and pure white.
    pub clipped_shadows: Option<f64>,
    pub clipped_highlights: Option<f64>,
}

impl Measurements {
    /// Millimetres per source pixel, fixed by the head occupying a known height.
    fn px_per_mm(&self, head_height_mm: f64) -> Option<f64> {
        if head_height_mm <= 0.0 || self.head_px <= 0.0 {
            return None;
        }
        Some(self.head_px / head_height_mm)
    }

    /// How tall the head will actually print, given the crop.
    fn printed_head_mm(&self, photo_height_mm: f64) -> Option<f64> {
        if self.crop.height <= 0.0 {
            return None;
        }
        Some(self.head_px / self.crop.height * photo_height_mm)
    }
}

/// Rules that are in the spec but cannot be measured with a five-point face
/// detector, each with the key explaining why.
///
/// `head_width` is the uncomfortable one: the spec marks it `error`, but it is
/// measured ear base to ear base and the detector reports only a bounding box,
/// which is not the width of the skull.
const NOT_MEASURABLE: &[(&str, &str)] = &[
    ("head_width", "rule.not_checked.head_width"),
    ("pose_yaw", "rule.not_checked.pose_yaw"),
    ("pose_pitch", "rule.not_checked.pose_pitch"),
    ("eyes_open", "rule.not_checked.eyes_open"),
    ("mouth_closed", "rule.not_checked.mouth_closed"),
    ("background_shadows", "rule.not_checked.background_shadows"),
    ("glasses_glare", "rule.not_checked.glasses_glare"),
    ("red_eye", "rule.not_checked.red_eye"),
    ("photo_age", "rule.not_checked.photo_age"),
];

fn not_measurable_key(rule_id: &str) -> Option<&'static str> {
    NOT_MEASURABLE.iter().find(|(id, _)| *id == rule_id).map(|(_, key)| *key)
}

/// Validate a photo against a spec.
///
/// `age_years` selects the age variant; `None` means adult.
/// `source_px` is the full source image size, for the resolution check.
pub fn validate(
    spec: &Spec,
    m: &Measurements,
    age_years: Option<f64>,
    dpi: f64,
) -> Vec<RuleResult> {
    let mut out = Vec::with_capacity(spec.rules.len());

    for rule in &spec.rules {
        // Anything the detector cannot see is reported honestly rather than
        // quietly passing.
        if let Some(key) = not_measurable_key(&rule.id) {
            out.push(RuleResult::new(
                &rule.id,
                rule.severity,
                Status::NotChecked,
                key,
            ));
            continue;
        }

        let result = match rule.id.as_str() {
            "head_height" => check_head_height(spec, m, age_years, rule.severity),
            "eye_distance" => check_eye_distance(spec, m, age_years, rule.severity),
            "horizontal_center" => check_centring(spec, m, age_years, rule.severity),
            "pose_roll" => check_roll(spec, m, rule.severity),
            "resolution_min" => check_resolution(spec, m, dpi, rule.severity),
            "background_uniform" => check_background(spec, m, rule.severity),
            "sharpness" => {
                check_sharpness(m, rule.number("min_laplacian_var").unwrap_or(100.0), rule.severity)
            }
            "exposure" => check_exposure(m, rule.severity),
            // An unrecognised rule id must not silently pass either.
            _ => Some(RuleResult::new(
                &rule.id,
                rule.severity,
                Status::NotChecked,
                "rule.not_checked.unknown",
            )),
        };

        if let Some(r) = result {
            out.push(r);
        }
    }

    out
}

fn check_head_height(
    spec: &Spec,
    m: &Measurements,
    age_years: Option<f64>,
    severity: Severity,
) -> Option<RuleResult> {
    let geometry = spec.geometry.as_ref()?;
    let variant = geometry.variant_for_age(age_years)?;
    let range = variant.head_height_mm;
    let printed = m.printed_head_mm(spec.print.height_mm)?;

    let params = serde_json::json!({
        "actual": round1(printed),
        "min": range.min,
        "max": range.max,
    });

    if range.contains(printed) {
        Some(
            RuleResult::new("head_height", severity, Status::Pass, "rule.head_height.ok")
                .with_params(params),
        )
    } else {
        let key = if printed < range.min {
            "rule.head_height.too_small"
        } else {
            "rule.head_height.too_large"
        };
        // Aim for the middle of the range rather than the nearest edge, so a
        // rounding wobble does not put it straight back outside.
        Some(
            RuleResult::new("head_height", severity, Status::Fail, key)
                .with_params(params)
                .with_fix(FixHint::SetHeadHeightMm(round1(range.midpoint()))),
        )
    }
}

fn check_eye_distance(
    spec: &Spec,
    m: &Measurements,
    age_years: Option<f64>,
    severity: Severity,
) -> Option<RuleResult> {
    let geometry = spec.geometry.as_ref()?;
    let limits = geometry.eye_distance_mm?;
    let variant = geometry.variant_for_age(age_years)?;
    let px_per_mm = m.px_per_mm(variant.head_height_mm.midpoint())?;
    if px_per_mm <= 0.0 {
        return None;
    }
    let eye_mm = m.eye_distance_px / px_per_mm;

    let params = serde_json::json!({ "actual": round1(eye_mm), "min": limits.min });
    if eye_mm >= limits.min {
        Some(
            RuleResult::new("eye_distance", severity, Status::Pass, "rule.eye_distance.ok")
                .with_params(params),
        )
    } else {
        Some(
            RuleResult::new(
                "eye_distance",
                severity,
                Status::Fail,
                "rule.eye_distance.too_small",
            )
            .with_params(params),
        )
    }
}

fn check_centring(
    spec: &Spec,
    m: &Measurements,
    age_years: Option<f64>,
    severity: Severity,
) -> Option<RuleResult> {
    let geometry = spec.geometry.as_ref()?;
    let centring = geometry.horizontal_centering?;
    if !centring.required {
        return None;
    }
    let variant = geometry.variant_for_age(age_years)?;
    let px_per_mm = m.px_per_mm(variant.head_height_mm.midpoint())?;
    if px_per_mm <= 0.0 {
        return None;
    }

    let offset_px = (m.head_centre.x - m.crop.center().x).abs();
    let offset_mm = offset_px / px_per_mm;

    let params =
        serde_json::json!({ "actual": round1(offset_mm), "tolerance": centring.tolerance_mm });
    let status = if offset_mm <= centring.tolerance_mm { Status::Pass } else { Status::Fail };
    let key = if status == Status::Pass {
        "rule.horizontal_center.ok"
    } else {
        "rule.horizontal_center.off"
    };
    Some(RuleResult::new("horizontal_center", severity, status, key).with_params(params))
}

fn check_roll(spec: &Spec, m: &Measurements, severity: Severity) -> Option<RuleResult> {
    let max = spec.pose.as_ref()?.max_roll_deg?;
    let actual = m.roll_deg.abs();
    let params = serde_json::json!({ "actual": round1(actual), "max": max });

    if actual <= max {
        Some(RuleResult::new("pose_roll", severity, Status::Pass, "rule.pose_roll.ok").with_params(params))
    } else {
        // The photo can simply be straightened, so offer that directly.
        Some(
            RuleResult::new("pose_roll", severity, Status::Fail, "rule.pose_roll.tilted")
                .with_params(params)
                .with_fix(FixHint::SetRotationDeg(round1(m.roll_deg))),
        )
    }
}

fn check_resolution(
    spec: &Spec,
    m: &Measurements,
    dpi: f64,
    severity: Severity,
) -> Option<RuleResult> {
    if m.crop.width <= 0.0 || m.crop.height <= 0.0 {
        return None;
    }
    // The DPI this crop supports without inventing pixels.
    let dpi_w = m.crop.width * 25.4 / spec.print.width_mm;
    let dpi_h = m.crop.height * 25.4 / spec.print.height_mm;
    let available = dpi_w.min(dpi_h);
    let required = spec.print.min_dpi.max(dpi);

    let params = serde_json::json!({
        "available": available.round(),
        "required": required.round(),
    });

    if available >= required {
        Some(
            RuleResult::new("resolution_min", severity, Status::Pass, "rule.resolution.ok")
                .with_params(params),
        )
    } else {
        Some(
            RuleResult::new("resolution_min", severity, Status::Fail, "rule.resolution.too_low")
                .with_params(params)
                .with_fix(FixHint::SetDpi(available.floor())),
        )
    }
}

fn check_background(spec: &Spec, m: &Measurements, severity: Severity) -> Option<RuleResult> {
    let max = spec.background.as_ref()?.uniformity_max_stddev?;
    // Without a mask there is nothing to measure; say so rather than pass.
    let Some(stddev) = m.background_stddev else {
        return Some(RuleResult::new(
            "background_uniform",
            severity,
            Status::NotChecked,
            "rule.not_checked.background_uniform",
        ));
    };

    let params = serde_json::json!({ "actual": round1(stddev), "max": max });
    let status = if stddev <= max { Status::Pass } else { Status::Fail };
    let key = if status == Status::Pass {
        "rule.background_uniform.ok"
    } else {
        "rule.background_uniform.uneven"
    };
    Some(RuleResult::new("background_uniform", severity, status, key).with_params(params))
}

fn check_sharpness(m: &Measurements, min_variance: f64, severity: Severity) -> Option<RuleResult> {
    let Some(value) = m.sharpness else {
        return Some(RuleResult::new(
            "sharpness",
            severity,
            Status::NotChecked,
            "rule.not_checked.sharpness",
        ));
    };
    let params = serde_json::json!({ "actual": round1(value), "min": min_variance });
    let status = if value >= min_variance { Status::Pass } else { Status::Warn };
    let key = if status == Status::Pass { "rule.sharpness.ok" } else { "rule.sharpness.soft" };
    Some(RuleResult::new("sharpness", severity, status, key).with_params(params))
}

fn check_exposure(m: &Measurements, severity: Severity) -> Option<RuleResult> {
    let (Some(shadows), Some(highlights)) = (m.clipped_shadows, m.clipped_highlights) else {
        return Some(RuleResult::new(
            "exposure",
            severity,
            Status::NotChecked,
            "rule.not_checked.exposure",
        ));
    };

    // A small amount of clipping is normal; a lot means detail is gone for good.
    const MAX_CLIPPED: f64 = 0.02;
    let params = serde_json::json!({
        "shadows": round1(shadows * 100.0),
        "highlights": round1(highlights * 100.0),
    });

    if shadows <= MAX_CLIPPED && highlights <= MAX_CLIPPED {
        Some(RuleResult::new("exposure", severity, Status::Pass, "rule.exposure.ok").with_params(params))
    } else {
        let key = if highlights > MAX_CLIPPED {
            "rule.exposure.overexposed"
        } else {
            "rule.exposure.underexposed"
        };
        Some(RuleResult::new("exposure", severity, Status::Warn, key).with_params(params))
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Whether anything failed at error severity, which is what gates auto-print.
pub fn has_blocking_failure(results: &[RuleResult]) -> bool {
    results
        .iter()
        .any(|r| r.status == Status::Fail && r.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::builtin_specs;

    fn passport() -> Spec {
        builtin_specs()
            .unwrap()
            .into_iter()
            .find(|s| s.id == "hr-passport-35x45")
            .unwrap()
    }

    /// Measurements for a head printing at exactly `head_mm` on a 35x45 photo.
    fn measurements_for(head_mm: f64) -> Measurements {
        let head_px = 700.0;
        // crop.height chosen so head_px/crop.height*45 == head_mm
        let crop_h = head_px / head_mm * 45.0;
        let crop_w = crop_h * 35.0 / 45.0;
        Measurements {
            head_px,
            crop: Rect::new(0.0, 0.0, crop_w, crop_h),
            head_centre: Point::new(crop_w / 2.0, crop_h / 2.0),
            // 0.32 of head height is a typical adult interpupillary distance.
            eye_distance_px: head_px * 0.32,
            roll_deg: 0.0,
            background_stddev: Some(2.0),
            sharpness: Some(250.0),
            clipped_shadows: Some(0.0),
            clipped_highlights: Some(0.0),
        }
    }

    fn find<'a>(results: &'a [RuleResult], id: &str) -> &'a RuleResult {
        results.iter().find(|r| r.rule_id == id).unwrap_or_else(|| panic!("no rule {id}"))
    }

    #[test]
    fn a_compliant_photo_passes_the_measurable_rules() {
        let results = validate(&passport(), &measurements_for(33.75), None, 300.0);
        for id in ["head_height", "eye_distance", "horizontal_center", "pose_roll"] {
            assert_eq!(find(&results, id).status, Status::Pass, "{id} should pass");
        }
    }

    #[test]
    fn a_head_below_the_range_fails_with_the_actual_number() {
        // 29.8mm against the official 31.5-36mm range.
        let results = validate(&passport(), &measurements_for(29.8), None, 300.0);
        let r = find(&results, "head_height");
        assert_eq!(r.status, Status::Fail);
        assert_eq!(r.message_key, "rule.head_height.too_small");
        assert_eq!(r.params["actual"], 29.8);
        assert_eq!(r.params["min"], 31.5);
        assert_eq!(r.params["max"], 36.0);
    }

    #[test]
    fn a_head_above_the_range_fails_the_other_way() {
        let results = validate(&passport(), &measurements_for(41.7), None, 300.0);
        let r = find(&results, "head_height");
        assert_eq!(r.status, Status::Fail);
        assert_eq!(r.message_key, "rule.head_height.too_large");
    }

    #[test]
    fn the_head_height_fix_actually_lands_in_range() {
        // A suggestion that still fails would be worse than none.
        let spec = passport();
        let results = validate(&spec, &measurements_for(29.8), None, 300.0);
        let Some(FixHint::SetHeadHeightMm(suggested)) = find(&results, "head_height").fix_hint
        else {
            panic!("expected a head height suggestion");
        };

        let range = spec
            .geometry
            .as_ref()
            .unwrap()
            .variant_for_age(None)
            .unwrap()
            .head_height_mm;
        assert!(
            range.contains(suggested),
            "suggested {suggested}mm is outside {}-{}mm",
            range.min,
            range.max
        );
    }

    #[test]
    fn a_tilted_head_fails_and_offers_the_correction() {
        let mut m = measurements_for(33.75);
        m.roll_deg = 8.0; // spec allows 5
        let results = validate(&passport(), &m, None, 300.0);
        let r = find(&results, "pose_roll");
        assert_eq!(r.status, Status::Fail);
        assert_eq!(r.fix_hint, Some(FixHint::SetRotationDeg(8.0)));
    }

    #[test]
    fn a_slight_tilt_within_tolerance_passes() {
        let mut m = measurements_for(33.75);
        m.roll_deg = -3.0;
        assert_eq!(find(&validate(&passport(), &m, None, 300.0), "pose_roll").status, Status::Pass);
    }

    #[test]
    fn an_off_centre_head_fails_centring() {
        let mut m = measurements_for(33.75);
        // Shift the head well beyond the 1mm tolerance.
        m.head_centre.x += 200.0;
        let results = validate(&passport(), &m, None, 300.0);
        let r = find(&results, "horizontal_center");
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn a_crop_too_small_for_300dpi_fails_resolution() {
        let mut m = measurements_for(33.75);
        // 35x45mm at 300dpi needs 413x531px; make the crop far smaller.
        m.crop = Rect::new(0.0, 0.0, 200.0, 257.0);
        m.head_px = 190.0;
        let results = validate(&passport(), &m, None, 300.0);
        let r = find(&results, "resolution_min");
        assert_eq!(r.status, Status::Fail);
        assert!(matches!(r.fix_hint, Some(FixHint::SetDpi(_))));
    }

    #[test]
    fn unmeasurable_rules_are_reported_as_not_checked() {
        // Never Pass: a green tick would promise a guarantee the program
        // cannot give.
        let results = validate(&passport(), &measurements_for(33.75), None, 300.0);
        for id in ["head_width", "pose_yaw", "pose_pitch", "eyes_open", "mouth_closed"] {
            let r = find(&results, id);
            assert_eq!(r.status, Status::NotChecked, "{id} must not claim to be checked");
            assert!(r.message_key.starts_with("rule.not_checked."));
        }
    }

    #[test]
    fn head_width_is_not_checked_despite_being_an_error_rule() {
        // The spec marks it error severity, but ear-to-ear width is not
        // derivable from a five-point detector.
        let results = validate(&passport(), &measurements_for(33.75), None, 300.0);
        let r = find(&results, "head_width");
        assert_eq!(r.severity, Severity::Error);
        assert_eq!(r.status, Status::NotChecked);
    }

    #[test]
    fn a_missing_mask_leaves_background_unchecked() {
        let mut m = measurements_for(33.75);
        m.background_stddev = None;
        let results = validate(&passport(), &m, None, 300.0);
        let r = find(&results, "background_uniform");
        assert_eq!(r.status, Status::NotChecked);
    }

    #[test]
    fn an_uneven_background_fails() {
        let mut m = measurements_for(33.75);
        m.background_stddev = Some(20.0); // spec allows 6
        let results = validate(&passport(), &m, None, 300.0);
        let r = find(&results, "background_uniform");
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn a_soft_photo_warns_rather_than_fails() {
        let mut m = measurements_for(33.75);
        m.sharpness = Some(40.0); // spec threshold is 100
        let results = validate(&passport(), &m, None, 300.0);
        let r = find(&results, "sharpness");
        assert_eq!(r.status, Status::Warn);
        assert_eq!(r.severity, Severity::Warning);
    }

    #[test]
    fn blown_highlights_warn() {
        let mut m = measurements_for(33.75);
        m.clipped_highlights = Some(0.15);
        let results = validate(&passport(), &m, None, 300.0);
        let r = find(&results, "exposure");
        assert_eq!(r.status, Status::Warn);
        assert_eq!(r.message_key, "rule.exposure.overexposed");
    }

    #[test]
    fn only_error_severity_failures_block() {
        let mut m = measurements_for(33.75);
        m.sharpness = Some(10.0); // a warning, not a blocker
        let results = validate(&passport(), &m, None, 300.0);
        assert!(!has_blocking_failure(&results), "a warning must not block");

        let results = validate(&passport(), &measurements_for(20.0), None, 300.0);
        assert!(has_blocking_failure(&results), "a failed head height must block");
    }

    #[test]
    fn the_child_variant_accepts_a_smaller_head() {
        // 24mm fails for an adult (min 31.5) but passes for a child (min 22.5).
        let m = measurements_for(24.0);
        assert_eq!(find(&validate(&passport(), &m, None, 300.0), "head_height").status, Status::Fail);
        assert_eq!(
            find(&validate(&passport(), &m, Some(5.0), 300.0), "head_height").status,
            Status::Pass
        );
    }

    #[test]
    fn a_free_mode_spec_produces_no_results() {
        let json = r#"{"specs":[{"id":"custom","print":{"width_mm":35,"height_mm":45}}]}"#;
        let spec = crate::spec::load_specs(json).unwrap().remove(0);
        assert!(validate(&spec, &measurements_for(33.75), None, 300.0).is_empty());
    }

    #[test]
    fn every_spec_rule_yields_exactly_one_result() {
        // No rule may be silently dropped: the panel must account for all of
        // them, checked or not.
        let spec = passport();
        let results = validate(&spec, &measurements_for(33.75), None, 300.0);
        assert_eq!(results.len(), spec.rules.len());
        for rule in &spec.rules {
            assert!(
                results.iter().any(|r| r.rule_id == rule.id),
                "rule {} produced no result",
                rule.id
            );
        }
    }
}
