//! Named sets of settings the user can save and recall.
//!
//! Stored in the same `config.toml` as the calibrations, under a separate table.
//! Deliberately not localStorage: the brief rules it out for anything that
//! matters, and a preset the user spent time building matters.
//!
//! A preset holds parameters only — never pixels, never a crop. The crop is
//! derived from the detected face and would be meaningless applied to a
//! different photo.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::calibration::CalibrationError;

/// Everything a preset remembers.
///
/// Every field is `#[serde(default)]` so a preset written by an older build
/// still loads after a field is added, rather than failing the whole file and
/// taking the calibrations down with it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Preset {
    /// Document specification id, empty for the free "custom" mode.
    #[serde(default)]
    pub spec_id: String,
    #[serde(default)]
    pub photo_width_mm: f64,
    #[serde(default)]
    pub photo_height_mm: f64,
    #[serde(default)]
    pub head_height_mm: f64,
    #[serde(default)]
    pub paper_id: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub margin_mm: f64,
    #[serde(default)]
    pub gutter_mm: f64,
    #[serde(default)]
    pub align_top_left: bool,
    #[serde(default)]
    pub cut_marks: bool,
    #[serde(default)]
    pub replace_background: bool,
    /// Background colour as RGB. Defaults to black rather than to a guessed
    /// grey, so a malformed entry is visible instead of plausibly wrong.
    #[serde(default)]
    pub background_rgb: [u8; 3],
    #[serde(default)]
    pub exposure_ev: f64,
    #[serde(default)]
    pub contrast: f64,
    #[serde(default)]
    pub temperature: f64,
    #[serde(default)]
    pub tint: f64,
    /// RFC3339 timestamp of the last save.
    #[serde(default)]
    pub saved_at: String,
}

/// All saved presets, keyed by the name the user typed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresetStore {
    #[serde(default)]
    presets: BTreeMap<String, Preset>,
}

/// The longest name accepted. Long enough for "Putovnica RH — mat papir",
/// short enough that the list stays readable.
const MAX_NAME_LEN: usize = 60;

impl PresetStore {
    pub fn get(&self, name: &str) -> Option<&Preset> {
        self.presets.get(name)
    }

    /// Save under `name`, replacing any preset already using it.
    ///
    /// Rejects empty or whitespace-only names: they would be invisible in the
    /// list and impossible to select again.
    pub fn set(&mut self, name: &str, value: Preset) -> Result<(), PresetError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(PresetError::EmptyName);
        }
        if trimmed.chars().count() > MAX_NAME_LEN {
            return Err(PresetError::NameTooLong { max: MAX_NAME_LEN });
        }
        self.presets.insert(trimmed.to_string(), value);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.presets.remove(name.trim()).is_some()
    }

    /// Names in sorted order, which is how the UI lists them.
    pub fn names(&self) -> Vec<String> {
        self.presets.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.presets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    EmptyName,
    NameTooLong { max: usize },
    NotFound,
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => write!(f, "a preset needs a name"),
            Self::NameTooLong { max } => write!(f, "preset name longer than {max} characters"),
            Self::NotFound => write!(f, "no preset by that name"),
        }
    }
}

impl std::error::Error for PresetError {}

/// Sheet settings that persist across sessions.
///
/// Set once in the settings window and then left alone: paper, margins and how
/// many copies go on a sheet do not change from photo to photo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetSettings {
    #[serde(default = "default_paper")]
    pub paper_id: String,
    #[serde(default = "default_count_setting")]
    pub count: u32,
    #[serde(default = "default_margin")]
    pub margin_mm: f64,
    #[serde(default = "default_gutter")]
    pub gutter_mm: f64,
    #[serde(default)]
    pub align_top_left: bool,
    #[serde(default = "default_true")]
    pub cut_marks: bool,
    /// Turn each photo frame on its side: 35x45 becomes 45x35.
    ///
    /// Off by default: photographs print upright, the way they are looked at.
    #[serde(default)]
    pub quarter_turn: bool,
    /// Turn the picture inside its frame. Independent of the frame's shape.
    #[serde(default)]
    pub turn_photo: bool,
    /// Printer chosen last time, restored on the next run.
    #[serde(default)]
    pub printer: String,
    /// UI text scale as a percentage. 100 is the design size.
    #[serde(default = "default_font_scale")]
    pub font_scale_percent: u32,
    /// Start with the window filling the screen.
    ///
    /// Maximised rather than true fullscreen: fullscreen hides the title bar
    /// and the close button, which is the wrong default for a desktop tool.
    #[serde(default)]
    pub start_maximized: bool,
    /// UI theme: "light" or "dark". Anything else is treated as light.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Chosen print resolution per printer, as "XxY" keyed by printer name.
    ///
    /// Per printer because the offered resolutions differ between devices, and
    /// a value carried over from another one would be silently wrong. An entry
    /// missing, or naming a resolution the driver no longer offers, means the
    /// driver's own current setting is used.
    #[serde(default)]
    pub printer_dpi: BTreeMap<String, String>,
}

fn default_theme() -> String {
    "light".to_string()
}

fn default_font_scale() -> u32 {
    100
}

/// Bounds for [`SheetSettings::font_scale_percent`], clamped on the way in so
/// a hand-edited config cannot make the UI unusable.
pub const FONT_SCALE_MIN: u32 = 50;
pub const FONT_SCALE_MAX: u32 = 200;

fn default_paper() -> String {
    "10x15".to_string()
}

fn default_count_setting() -> u32 {
    6
}

fn default_margin() -> f64 {
    3.0
}

fn default_gutter() -> f64 {
    2.0
}

fn default_true() -> bool {
    true
}

impl Default for SheetSettings {
    fn default() -> Self {
        Self {
            paper_id: default_paper(),
            count: default_count_setting(),
            margin_mm: default_margin(),
            gutter_mm: default_gutter(),
            align_top_left: false,
            cut_marks: true,
            quarter_turn: false,
            turn_photo: false,
            printer: String::new(),
            font_scale_percent: default_font_scale(),
            start_maximized: false,
            theme: default_theme(),
            printer_dpi: BTreeMap::new(),
        }
    }
}

/// The whole config file: calibrations, presets and sheet settings side by side.
///
/// Both live in one file because they are both per-machine settings, and one
/// atomic write is easier to reason about than two. Loading and saving goes
/// through this type so that saving a preset cannot drop the calibrations, the
/// classic bug when two features own the same file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Calibration entries, kept in the shape the existing store writes so old
    /// config files keep loading.
    #[serde(default)]
    pub entries: BTreeMap<String, crate::calibration::Calibration>,
    #[serde(default)]
    pub presets: BTreeMap<String, Preset>,
    #[serde(default)]
    pub sheet: SheetSettings,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, CalibrationError> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| CalibrationError::Parse(e.to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CalibrationError::Io(e.to_string())),
        }
    }

    /// Write atomically: temp file then rename, so an interrupted write cannot
    /// leave a truncated config that loses both features at once.
    pub fn save(&self, path: &Path) -> Result<(), CalibrationError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CalibrationError::Io(e.to_string()))?;
        }
        let toml = toml::to_string_pretty(self).map_err(|e| CalibrationError::Parse(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml).map_err(|e| CalibrationError::Io(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| CalibrationError::Io(e.to_string()))
    }

    pub fn store(&self) -> PresetStore {
        PresetStore { presets: self.presets.clone() }
    }

    pub fn set_store(&mut self, store: PresetStore) {
        self.presets = store.presets;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::Calibration;

    const NOW: &str = "2026-08-09T12:00:00Z";

    fn sample() -> Preset {
        Preset {
            spec_id: "hr-passport-35x45".into(),
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.75,
            paper_id: "10x15".into(),
            count: 8,
            margin_mm: 3.0,
            gutter_mm: 2.0,
            align_top_left: false,
            cut_marks: true,
            replace_background: true,
            background_rgb: [235, 235, 235],
            exposure_ev: 0.25,
            contrast: 0.1,
            temperature: -0.05,
            tint: 0.0,
            saved_at: NOW.into(),
        }
    }

    #[test]
    fn a_preset_round_trips_through_toml() {
        let mut cfg = Config::default();
        let mut store = cfg.store();
        store.set("Putovnica", sample()).unwrap();
        cfg.set_store(store);

        let toml = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&toml).unwrap();
        assert_eq!(back.store().get("Putovnica"), Some(&sample()));
    }

    #[test]
    fn saving_a_preset_keeps_the_calibrations() {
        // The two features share one file. Writing one must not drop the other,
        // which would silently undo a calibration the user measured by hand.
        let mut cfg = Config::default();
        cfg.entries.insert(
            "P|210000x297000|false".into(),
            Calibration::from_measurement(50.0, 49.8, 50.1, NOW).unwrap(),
        );

        let mut store = cfg.store();
        store.set("Putovnica", sample()).unwrap();
        cfg.set_store(store);

        let toml = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&toml).unwrap();
        assert_eq!(back.entries.len(), 1, "calibration was lost");
        assert_eq!(back.presets.len(), 1, "preset was lost");
    }

    #[test]
    fn sheet_settings_round_trip_and_default_sensibly() {
        let mut cfg = Config::default();
        assert_eq!(cfg.sheet.paper_id, "10x15", "unset paper should default");
        assert!(!cfg.sheet.quarter_turn, "photos print upright by default");
        assert!(!cfg.sheet.turn_photo, "the picture is not turned by default");

        cfg.sheet.count = 8;
        cfg.sheet.printer = "Brother HL-L2402D".into();
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.sheet.count, 8);
        assert_eq!(back.sheet.printer, "Brother HL-L2402D");
    }

    #[test]
    fn font_scale_defaults_to_one_hundred_percent() {
        // A config written before this setting existed must not come back with
        // a zero scale, which would render the UI invisible.
        let old = r#"
[sheet]
paper_id = "10x15"
count = 6
"#;
        let cfg: Config = toml::from_str(old).unwrap();
        assert_eq!(cfg.sheet.font_scale_percent, 100);
    }

    #[test]
    fn theme_round_trips_and_defaults_to_light() {
        // A config written before the theme existed must not come back with an
        // empty string, which would render as an unstyled interface.
        let old: Config = toml::from_str("[sheet]\npaper_id = \"10x15\"\n").unwrap();
        assert_eq!(old.sheet.theme, "light");

        let mut cfg = Config::default();
        cfg.sheet.theme = "dark".into();
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.sheet.theme, "dark");
    }

    #[test]
    fn start_maximized_round_trips_and_defaults_off() {
        // Startup reads this before the window exists, so a config written
        // before the setting existed must yield a usable default rather than
        // failing to parse.
        let old: Config = toml::from_str("[sheet]\npaper_id = \"10x15\"\n").unwrap();
        assert!(!old.sheet.start_maximized);

        let mut cfg = Config::default();
        cfg.sheet.start_maximized = true;
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert!(back.sheet.start_maximized);
    }

    #[test]
    fn font_scale_round_trips() {
        let mut cfg = Config::default();
        cfg.sheet.font_scale_percent = 150;
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.sheet.font_scale_percent, 150);
    }

    #[test]
    fn a_config_written_before_sheet_settings_existed_still_loads() {
        // Same guarantee as for presets: an older file must not fail to parse
        // and take the calibrations down with it.
        let old = r#"
[entries."P|210000x297000|false"]
scale_x = 1.0
scale_y = 1.0
offset_x_mm = 0.0
offset_y_mm = 0.0
calibrated_at = "2026-08-07T19:00:00Z"
"#;
        let cfg: Config = toml::from_str(old).unwrap();
        assert_eq!(cfg.entries.len(), 1);
        assert_eq!(cfg.sheet, SheetSettings::default());
    }

    #[test]
    fn a_config_with_only_calibrations_still_loads() {
        // Files written before presets existed have no [presets] table at all.
        let old = r#"
[entries."P|210000x297000|false"]
scale_x = 1.004
scale_y = 0.998
offset_x_mm = 0.0
offset_y_mm = 0.0
calibrated_at = "2026-08-07T19:00:00Z"
"#;
        let cfg: Config = toml::from_str(old).unwrap();
        assert_eq!(cfg.entries.len(), 1);
        assert!(cfg.presets.is_empty());
    }

    #[test]
    fn nameless_presets_are_rejected() {
        let mut store = PresetStore::default();
        assert_eq!(store.set("", sample()), Err(PresetError::EmptyName));
        assert_eq!(store.set("   ", sample()), Err(PresetError::EmptyName));
        assert!(store.is_empty());
    }

    #[test]
    fn names_are_trimmed_so_one_preset_cannot_hide_another() {
        let mut store = PresetStore::default();
        store.set("Putovnica", sample()).unwrap();
        store.set("  Putovnica  ", sample()).unwrap();
        assert_eq!(store.len(), 1, "whitespace created a duplicate");
    }

    #[test]
    fn an_overlong_name_is_rejected() {
        let mut store = PresetStore::default();
        let long = "a".repeat(MAX_NAME_LEN + 1);
        assert_eq!(store.set(&long, sample()), Err(PresetError::NameTooLong { max: MAX_NAME_LEN }));
    }

    #[test]
    fn saving_under_an_existing_name_replaces_it() {
        let mut store = PresetStore::default();
        store.set("A", sample()).unwrap();
        let mut changed = sample();
        changed.count = 99;
        store.set("A", changed.clone()).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("A").unwrap().count, 99);
    }

    #[test]
    fn removing_reports_whether_anything_went() {
        let mut store = PresetStore::default();
        store.set("A", sample()).unwrap();
        assert!(store.remove("A"));
        assert!(!store.remove("A"));
    }

    #[test]
    fn names_come_back_sorted() {
        let mut store = PresetStore::default();
        for n in ["Viza", "Osobna", "Putovnica"] {
            store.set(n, sample()).unwrap();
        }
        assert_eq!(store.names(), vec!["Osobna", "Putovnica", "Viza"]);
    }

    #[test]
    fn a_preset_survives_a_real_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("presets-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let _ = std::fs::remove_dir_all(&dir);

        let mut cfg = Config::default();
        let mut store = cfg.store();
        store.set("Putovnica", sample()).unwrap();
        cfg.set_store(store);
        cfg.save(&path).unwrap();

        let back = Config::load(&path).unwrap();
        assert_eq!(back.store().get("Putovnica"), Some(&sample()));

        // The temp file must not be left behind next to the config.
        assert!(!path.with_extension("toml.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `Config` and `CalibrationStore` both serialise the calibrations under
    /// `entries`. Nothing but this test makes them agree: rename the field in
    /// either type and a saved calibration would quietly vanish the next time a
    /// preset is written. Round-trip through the *other* type to prove it.
    #[test]
    fn config_and_calibration_store_agree_on_the_file_format() {
        use crate::calibration::{CalibrationKey, CalibrationStore};

        let key = CalibrationKey::new("Brother HL-L2402D", 210.0, 297.0, false);
        let mut store = CalibrationStore::default();
        store.set(&key, Calibration::from_measurement(50.0, 49.8, 50.1, NOW).unwrap());

        // Written by the calibration store, read by Config.
        let cfg: Config = toml::from_str(&store.to_toml().unwrap()).unwrap();
        assert_eq!(cfg.entries.len(), 1, "Config cannot see the calibration store's entries");

        // And back the other way, which is what happens after a preset is saved.
        let round = CalibrationStore::from_toml(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert!(round.get(&key).is_some(), "the calibration store lost the entry");
    }

    #[test]
    fn a_missing_file_loads_as_empty() {
        let path = std::env::temp_dir().join("definitely-not-a-config-9f3a.toml");
        let _ = std::fs::remove_file(&path);
        let cfg = Config::load(&path).unwrap();
        assert!(cfg.presets.is_empty());
        assert!(cfg.entries.is_empty());
    }
}
