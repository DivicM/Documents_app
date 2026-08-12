//! Document specifications: print size, crop geometry, and which rules apply.
//!
//! Specs are data, not code (§8 of the brief). They are deserialised from JSON
//! and carry only two things that affect behaviour: the starting geometry of the
//! crop, and which rules get validated. Import, background removal, layout and
//! printing are the same code whichever spec is selected.
//!
//! Every length is millimetres. Conversion to pixels happens once, in `units`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a spec measures the crop from.
///
/// An enum rather than a string because countries genuinely differ here and
/// adding a variant later would mean migrating every spec file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Measured upward from the chin line, as the Croatian template does.
    ChinLine,
    /// Measured from the bottom edge to the eye line, common elsewhere.
    EyeLine,
    /// Head simply centred in the frame.
    Frame,
}

/// What "head height" means for a given spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadMeasure {
    /// Chin to the top of the skull, hair may exceed it.
    ChinToCrown,
    /// Chin to the top of the hair.
    ChinToTopOfHair,
}

/// How much the numbers in a spec can be trusted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Taken from a named regulation with a source URL.
    Verified,
    /// Widely used values without a single authoritative source.
    Baseline,
    /// Contributed, unverified. Assumed when a spec does not say.
    #[default]
    Community,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

impl Range {
    pub fn contains(&self, v: f64) -> bool {
        v >= self.min && v <= self.max
    }

    pub fn midpoint(&self) -> f64 {
        (self.min + self.max) / 2.0
    }
}

/// Measurements for one age group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgeVariant {
    pub id: String,
    #[serde(default)]
    pub age_min_years: Option<f64>,
    #[serde(default)]
    pub age_max_years: Option<f64>,
    pub head_height_mm: Range,
    #[serde(default)]
    pub head_width_mm: Option<Range>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EyeDistance {
    pub min: f64,
    #[serde(default)]
    pub optimal: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorizontalCentering {
    pub required: bool,
    pub tolerance_mm: f64,
}

/// A value the regulation does not state numerically.
///
/// Modelled explicitly rather than as a bare `Option` so the reason survives
/// into the code: the Croatian chin line exists only as a graphic on the MUP
/// template, and the spec says to fall back to head height plus centring until
/// somebody measures it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeasuredValue {
    #[serde(default)]
    pub value: Option<f64>,
}

impl MeasuredValue {
    pub fn get(&self) -> Option<f64> {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub anchor: Anchor,
    pub head_measure: HeadMeasure,
    pub age_variants: Vec<AgeVariant>,
    #[serde(default)]
    pub eye_distance_mm: Option<EyeDistance>,
    #[serde(default)]
    pub horizontal_centering: Option<HorizontalCentering>,
    /// Distance from the bottom edge up to the chin line. Usually absent.
    #[serde(default)]
    pub chin_line_from_bottom_mm: Option<MeasuredValue>,
}

impl Geometry {
    /// The variant matching an age, or the first one as a fallback.
    ///
    /// `None` selects the adult variant, which is what an unspecified age
    /// should mean: most subjects are adults, and the child ranges are wider,
    /// so defaulting to them would pass photos an adult spec would reject.
    pub fn variant_for_age(&self, age_years: Option<f64>) -> Option<&AgeVariant> {
        match age_years {
            Some(age) => self
                .age_variants
                .iter()
                .find(|v| {
                    v.age_min_years.map(|m| age >= m).unwrap_or(true)
                        && v.age_max_years.map(|m| age < m).unwrap_or(true)
                })
                .or_else(|| self.age_variants.first()),
            None => self
                .age_variants
                .iter()
                .find(|v| v.id == "adult")
                .or_else(|| self.age_variants.first()),
        }
    }

    /// The chin line if the spec actually states one.
    ///
    /// Feeds `CropTarget::chin_from_bottom_mm` directly, where `None` already
    /// means "centre the head vertically".
    pub fn chin_from_bottom_mm(&self) -> Option<f64> {
        self.chin_line_from_bottom_mm.and_then(|m| m.get())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrintTarget {
    pub width_mm: f64,
    pub height_mm: f64,
    #[serde(default = "default_min_dpi")]
    pub min_dpi: f64,
    #[serde(default)]
    pub preferred_dpi: Option<f64>,
}

fn default_min_dpi() -> f64 {
    300.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutDefaults {
    #[serde(default)]
    pub paper: Option<String>,
    #[serde(default = "default_count")]
    pub count: u32,
    #[serde(default)]
    pub cut_marks: bool,
}

fn default_count() -> u32 {
    1
}

impl Default for LayoutDefaults {
    fn default() -> Self {
        Self { paper: None, count: 1, cut_marks: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// One entry from a spec's `rules` array.
///
/// Rules are not homogeneous: `sharpness` carries its own threshold, so extra
/// keys are captured rather than dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConfig {
    pub id: String,
    pub severity: Severity,
    #[serde(flatten, default)]
    pub params: BTreeMap<String, serde_json::Value>,
}

impl RuleConfig {
    /// A numeric parameter such as `min_laplacian_var`.
    pub fn number(&self, key: &str) -> Option<f64> {
        self.params.get(key).and_then(|v| v.as_f64())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Background {
    #[serde(default)]
    pub allowed: Vec<AllowedColour>,
    #[serde(default)]
    pub uniformity_max_stddev: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowedColour {
    pub name: String,
    pub rgb: [u8; 3],
    #[serde(default)]
    pub tolerance: Option<f64>,
    #[serde(default)]
    pub default: bool,
}

impl Background {
    /// The colour a spec prefers, or light grey if it names none.
    pub fn default_rgb(&self) -> [u8; 3] {
        self.allowed
            .iter()
            .find(|c| c.default)
            .or_else(|| self.allowed.first())
            .map(|c| c.rgb)
            .unwrap_or([235, 235, 235])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    #[serde(default)]
    pub max_roll_deg: Option<f64>,
    #[serde(default)]
    pub max_yaw_deg: Option<f64>,
    #[serde(default)]
    pub max_pitch_deg: Option<f64>,
}

/// A fully resolved spec, with any `inherits` already merged in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub id: String,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub document: Option<String>,
    #[serde(default)]
    pub display_name: BTreeMap<String, String>,
    pub print: PrintTarget,
    /// `None` means free crop with no guides.
    #[serde(default)]
    pub geometry: Option<Geometry>,
    #[serde(default)]
    pub pose: Option<Pose>,
    #[serde(default)]
    pub background: Option<Background>,
    /// Empty means no checks at all.
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
    #[serde(default)]
    pub layout_defaults: LayoutDefaults,
    #[serde(default)]
    pub confidence: Confidence,
}

impl Spec {
    /// Name in the requested language, falling back to the id.
    pub fn name(&self, lang: &str) -> String {
        self.display_name
            .get(lang)
            .or_else(|| self.display_name.get("en"))
            .cloned()
            .unwrap_or_else(|| self.id.clone())
    }

    /// Whether this spec constrains the crop at all.
    pub fn is_free_mode(&self) -> bool {
        self.geometry.is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpecError {
    Parse(String),
    /// An `inherits` pointed at a spec that is not in the file.
    UnknownParent { spec: String, parent: String },
    /// Inheritance formed a cycle.
    CircularInheritance(String),
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(m) => write!(f, "could not parse specs: {m}"),
            Self::UnknownParent { spec, parent } => {
                write!(f, "spec {spec} inherits from unknown spec {parent}")
            }
            Self::CircularInheritance(id) => write!(f, "circular inheritance at {id}"),
        }
    }
}

impl std::error::Error for SpecError {}

/// Load and resolve every spec in a JSON document.
///
/// Specs may be partial overlays naming a parent via `inherits`; this merges
/// them so callers only ever see complete specs.
pub fn load_specs(json: &str) -> Result<Vec<Spec>, SpecError> {
    let doc: serde_json::Value =
        serde_json::from_str(json).map_err(|e| SpecError::Parse(e.to_string()))?;

    let raw = doc
        .get("specs")
        .and_then(|s| s.as_array())
        .ok_or_else(|| SpecError::Parse("missing `specs` array".into()))?;

    // Index by id so parents can be found regardless of file order.
    let mut by_id: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for entry in raw {
        let id = entry
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SpecError::Parse("a spec has no id".into()))?
            .to_string();
        order.push(id.clone());
        by_id.insert(id, entry.clone());
    }

    let mut out = Vec::with_capacity(order.len());
    for id in &order {
        let resolved = resolve(id, &by_id, &mut Vec::new())?;
        let spec: Spec =
            serde_json::from_value(resolved).map_err(|e| SpecError::Parse(format!("{id}: {e}")))?;
        out.push(spec);
    }
    Ok(out)
}

/// Merge a spec onto its parent chain.
fn resolve(
    id: &str,
    by_id: &BTreeMap<String, serde_json::Value>,
    seen: &mut Vec<String>,
) -> Result<serde_json::Value, SpecError> {
    if seen.iter().any(|s| s == id) {
        return Err(SpecError::CircularInheritance(id.to_string()));
    }
    seen.push(id.to_string());

    let entry = by_id
        .get(id)
        .ok_or_else(|| SpecError::UnknownParent { spec: seen.join(" -> "), parent: id.into() })?;

    let Some(parent_id) = entry.get("inherits").and_then(|v| v.as_str()) else {
        return Ok(entry.clone());
    };
    if !by_id.contains_key(parent_id) {
        return Err(SpecError::UnknownParent {
            spec: id.to_string(),
            parent: parent_id.to_string(),
        });
    }

    let parent = resolve(parent_id, by_id, seen)?;
    let mut merged = parent;
    merge(&mut merged, entry);
    // The child's own identity must survive the merge.
    if let Some(obj) = merged.as_object_mut() {
        obj.insert("id".into(), serde_json::Value::String(id.to_string()));
        obj.remove("inherits");
    }
    Ok(merged)
}

/// Recursively overlay `patch` onto `base`.
///
/// Objects merge key by key so a child can override one field without
/// restating the section; anything else replaces wholesale, which is what an
/// overridden array such as `age_variants` should do.
fn merge(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(existing) => merge(existing, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// The Croatian specs, compiled into the binary.
///
/// Embedded rather than read from disk so the app cannot be broken by a missing
/// file and the offline guarantee stays trivial.
pub const HR_SPECS_JSON: &str = include_str!("../../../specs/hr.json");

/// Widely used non-Croatian formats. Kept in a separate file because none of
/// them carries a verified source, and mixing them with the Croatian specs
/// would blur that distinction.
pub const INTERNATIONAL_SPECS_JSON: &str = include_str!("../../../specs/international.json");

/// Every spec the app ships with, Croatian ones first.
///
/// `inherits` is resolved per file, so an international spec cannot inherit
/// from a Croatian one. That is deliberate: the two sets have different
/// provenance and should not silently share numbers.
pub fn builtin_specs() -> Result<Vec<Spec>, SpecError> {
    let mut specs = load_specs(HR_SPECS_JSON)?;
    specs.extend(load_specs(INTERNATIONAL_SPECS_JSON)?);
    Ok(specs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_croatian_specs_load() {
        let specs = builtin_specs().expect("bundled specs must parse");
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"hr-passport-35x45"));
        assert!(ids.contains(&"hr-putni-list-30x35"));
        assert!(ids.contains(&"hr-intl-treaty-35x45"));
    }

    #[test]
    fn the_international_specs_load() {
        let specs = builtin_specs().expect("bundled specs must parse");
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"us-visa-51x51"));
        assert!(ids.contains(&"schengen-visa-35x45"));
        assert!(ids.contains(&"generic-25x30"));
        assert!(ids.contains(&"generic-30x40"));
        assert!(ids.contains(&"generic-40x60"));
        assert!(ids.contains(&"free-custom"));
    }

    /// The generic formats are upright, correctly sized and carry no geometry.
    ///
    /// Both halves matter: swapping the dimensions would lay out a landscape
    /// frame, and adding geometry would invent a head-height range no
    /// regulation backs, which the picker would then present as a real check.
    #[test]
    fn the_generic_formats_are_upright_and_unchecked() {
        let specs = builtin_specs().unwrap();
        for (id, w, h) in [
            ("generic-25x30", 25.0, 30.0),
            ("generic-30x40", 30.0, 40.0),
            ("generic-40x60", 40.0, 60.0),
        ] {
            let spec = specs.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no {id}"));
            assert_eq!(spec.print.width_mm, w, "{id} width");
            assert_eq!(spec.print.height_mm, h, "{id} height");
            assert!(spec.print.height_mm > spec.print.width_mm, "{id} must be portrait");
            assert!(spec.is_free_mode(), "{id}: no geometry, so cropping is free");
            assert!(spec.rules.is_empty(), "{id}: no source means no checks");
        }
    }

    /// The whole point of splitting the files: a spec without a named
    /// regulation must never claim to be verified, because the UI uses this to
    /// tell the user which numbers to double-check.
    #[test]
    fn only_the_croatian_specs_claim_to_be_verified() {
        for spec in builtin_specs().unwrap() {
            if spec.id.starts_with("hr-") {
                continue;
            }
            assert_ne!(
                spec.confidence,
                Confidence::Verified,
                "{} claims verified without a regulation behind it",
                spec.id
            );
        }
    }

    #[test]
    fn the_us_format_is_square() {
        // A square target is the one shape most likely to expose a bug in code
        // that quietly assumes portrait.
        let s = spec("us-visa-51x51");
        assert_eq!(s.print.width_mm, s.print.height_mm);
    }

    fn spec(id: &str) -> Spec {
        builtin_specs()
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no spec {id}"))
    }

    #[test]
    fn passport_carries_the_official_numbers() {
        let s = spec("hr-passport-35x45");
        assert_eq!(s.print.width_mm, 35.0);
        assert_eq!(s.print.height_mm, 45.0);

        let g = s.geometry.expect("passport constrains geometry");
        assert_eq!(g.anchor, Anchor::ChinLine);
        assert_eq!(g.head_measure, HeadMeasure::ChinToCrown);

        let adult = g.variant_for_age(None).expect("adult variant");
        assert_eq!(adult.id, "adult");
        assert_eq!(adult.head_height_mm.min, 31.5);
        assert_eq!(adult.head_height_mm.max, 36.0);
    }

    #[test]
    fn the_unmeasured_chin_line_stays_absent() {
        // The regulation defines it graphically with no number, so the spec has
        // `null`. Reporting a value here would invent a measurement.
        let g = spec("hr-passport-35x45").geometry.unwrap();
        assert_eq!(
            g.chin_from_bottom_mm(),
            None,
            "an unmeasured chin line must not become a number"
        );
    }

    #[test]
    fn putni_list_overrides_size_but_inherits_the_rest() {
        let s = spec("hr-putni-list-30x35");
        assert_eq!(s.print.width_mm, 30.0);
        assert_eq!(s.print.height_mm, 35.0);

        // Its own head heights.
        let g = s.geometry.expect("inherited geometry");
        let adult = g.variant_for_age(None).unwrap();
        assert_eq!(adult.head_height_mm.min, 24.5);

        // Inherited from the passport: anchor, rules, pose.
        assert_eq!(g.anchor, Anchor::ChinLine);
        assert!(!s.rules.is_empty(), "rules should come from the parent");
        assert_eq!(s.layout_defaults.count, 8, "its own layout default");
    }

    #[test]
    fn treaty_spec_inherits_almost_everything() {
        // It overrides only `legal`, so every other field must come through.
        let s = spec("hr-intl-treaty-35x45");
        assert_eq!(s.print.width_mm, 35.0);
        assert_eq!(s.print.height_mm, 45.0);
        let g = s.geometry.expect("geometry must be inherited");
        assert_eq!(g.variant_for_age(None).unwrap().head_height_mm.min, 31.5);
        assert_eq!(s.layout_defaults.count, 6, "inherited layout default");
    }

    #[test]
    fn rules_keep_their_extra_parameters() {
        let s = spec("hr-passport-35x45");
        let sharpness = s
            .rules
            .iter()
            .find(|r| r.id == "sharpness")
            .expect("sharpness rule present");
        assert_eq!(sharpness.severity, Severity::Warning);
        assert_eq!(
            sharpness.number("min_laplacian_var"),
            Some(100.0),
            "per-rule parameters must survive deserialisation"
        );
    }

    #[test]
    fn child_variant_is_selected_by_age() {
        let g = spec("hr-passport-35x45").geometry.unwrap();
        let child = g.variant_for_age(Some(6.0)).unwrap();
        assert_eq!(child.id, "child");
        assert_eq!(child.head_height_mm.min, 22.5);

        let adult = g.variant_for_age(Some(30.0)).unwrap();
        assert_eq!(adult.id, "adult");
    }

    #[test]
    fn unspecified_age_uses_the_adult_variant() {
        // Child ranges are wider, so defaulting to them would pass photos the
        // adult spec rejects.
        let g = spec("hr-passport-35x45").geometry.unwrap();
        assert_eq!(g.variant_for_age(None).unwrap().id, "adult");
    }

    #[test]
    fn background_default_matches_the_guidance() {
        let bg = spec("hr-passport-35x45").background.expect("background section");
        assert_eq!(bg.default_rgb(), [235, 235, 235], "light grey is the default");
    }

    #[test]
    fn free_mode_spec_has_no_geometry_or_rules() {
        let json = r#"{
            "specs": [{
                "id": "custom",
                "print": { "width_mm": 35, "height_mm": 45 }
            }]
        }"#;
        let specs = load_specs(json).unwrap();
        assert!(specs[0].is_free_mode());
        assert!(specs[0].rules.is_empty());
    }

    #[test]
    fn an_unknown_anchor_is_rejected() {
        // Better to fail loudly than to silently treat an unrecognised anchor
        // as one of the known ones.
        let json = r#"{
            "specs": [{
                "id": "bad",
                "print": { "width_mm": 35, "height_mm": 45 },
                "geometry": {
                    "anchor": "navel",
                    "head_measure": "chin_to_crown",
                    "age_variants": []
                }
            }]
        }"#;
        assert!(matches!(load_specs(json), Err(SpecError::Parse(_))));
    }

    #[test]
    fn inheriting_from_a_missing_spec_is_an_error() {
        let json = r#"{
            "specs": [{
                "id": "orphan",
                "inherits": "nonexistent",
                "print": { "width_mm": 35, "height_mm": 45 }
            }]
        }"#;
        assert!(matches!(load_specs(json), Err(SpecError::UnknownParent { .. })));
    }

    #[test]
    fn circular_inheritance_is_caught() {
        let json = r#"{
            "specs": [
                { "id": "a", "inherits": "b", "print": { "width_mm": 35, "height_mm": 45 } },
                { "id": "b", "inherits": "a", "print": { "width_mm": 35, "height_mm": 45 } }
            ]
        }"#;
        assert!(matches!(load_specs(json), Err(SpecError::CircularInheritance(_))));
    }

    #[test]
    fn display_name_falls_back_to_the_id() {
        let json = r#"{
            "specs": [{ "id": "nameless", "print": { "width_mm": 35, "height_mm": 45 } }]
        }"#;
        let specs = load_specs(json).unwrap();
        assert_eq!(specs[0].name("hr"), "nameless");
    }

    #[test]
    fn croatian_display_names_are_present() {
        assert_eq!(spec("hr-passport-35x45").name("hr"), "Dokumenti (RH)");
    }
}
