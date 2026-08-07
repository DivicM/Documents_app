//! Per-printer calibration: what the machine actually puts on paper versus
//! what it was asked to.
//!
//! Every printer is slightly off, and the error differs between the paper-feed
//! direction and the print-head direction, so X and Y are stored separately.
//! Calibration is keyed by paper size and borderless mode too, because both
//! change the transport path.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Identifies one calibration. Two printers of the same model, or the same
/// printer with different paper, do not share a correction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CalibrationKey {
    pub printer_name: String,
    /// Paper size in whole micrometres, so the key can be Eq/Ord. Floats would
    /// make 100.0 and 100.00000001 different keys.
    pub paper_width_um: u32,
    pub paper_height_um: u32,
    pub borderless: bool,
}

impl CalibrationKey {
    pub fn new(printer_name: &str, paper_width_mm: f64, paper_height_mm: f64, borderless: bool) -> Self {
        Self {
            printer_name: printer_name.to_string(),
            paper_width_um: (paper_width_mm * 1000.0).round() as u32,
            paper_height_um: (paper_height_mm * 1000.0).round() as u32,
            borderless,
        }
    }

    /// Flat string form, used as the TOML table key.
    fn to_storage_key(&self) -> String {
        format!(
            "{}|{}x{}|{}",
            self.printer_name, self.paper_width_um, self.paper_height_um, self.borderless
        )
    }
}

/// Correction factors measured from a test print.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Multiply requested width by this to get what the printer actually does.
    pub scale_x: f64,
    pub scale_y: f64,
    pub offset_x_mm: f64,
    pub offset_y_mm: f64,
    /// RFC3339 timestamp, so the UI can show how stale the calibration is.
    pub calibrated_at: String,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            scale_x: 1.0,
            scale_y: 1.0,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
            calibrated_at: String::new(),
        }
    }
}

impl Calibration {
    /// Derive a calibration from a measured test square.
    ///
    /// `nominal_mm` is what was asked for (50mm), `measured_*` what the ruler
    /// shows. A square printing at 49.5mm means the printer runs 1% small, so
    /// subsequent output is scaled up to compensate.
    pub fn from_measurement(
        nominal_mm: f64,
        measured_x_mm: f64,
        measured_y_mm: f64,
        now_rfc3339: &str,
    ) -> Result<Self, CalibrationError> {
        if nominal_mm <= 0.0 {
            return Err(CalibrationError::InvalidMeasurement("nominal size must be positive"));
        }
        if measured_x_mm <= 0.0 || measured_y_mm <= 0.0 {
            return Err(CalibrationError::InvalidMeasurement("measured size must be positive"));
        }

        let scale_x = nominal_mm / measured_x_mm;
        let scale_y = nominal_mm / measured_y_mm;

        // A correction beyond 10% means the square was mismeasured or the wrong
        // paper was used. Silently accepting it would bake in a bad number.
        const MAX_DEVIATION: f64 = 0.10;
        if (scale_x - 1.0).abs() > MAX_DEVIATION || (scale_y - 1.0).abs() > MAX_DEVIATION {
            return Err(CalibrationError::ImplausibleScale { scale_x, scale_y });
        }

        Ok(Self {
            scale_x,
            scale_y,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
            calibrated_at: now_rfc3339.to_string(),
        })
    }

    /// Whether this is the identity, i.e. nothing has been calibrated yet.
    pub fn is_uncalibrated(&self) -> bool {
        self.calibrated_at.is_empty()
    }

    /// Apply the correction to a length in millimetres.
    pub fn apply_x(&self, mm: f64) -> f64 {
        mm * self.scale_x
    }

    pub fn apply_y(&self, mm: f64) -> f64 {
        mm * self.scale_y
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CalibrationError {
    InvalidMeasurement(&'static str),
    ImplausibleScale { scale_x: f64, scale_y: f64 },
    Io(String),
    Parse(String),
}

impl std::fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMeasurement(m) => write!(f, "invalid measurement: {m}"),
            Self::ImplausibleScale { scale_x, scale_y } => write!(
                f,
                "implausible correction (x {scale_x:.4}, y {scale_y:.4}); \
                 check the ruler reading and the paper size"
            ),
            Self::Io(m) => write!(f, "config io error: {m}"),
            Self::Parse(m) => write!(f, "config parse error: {m}"),
        }
    }
}

impl std::error::Error for CalibrationError {}

/// All stored calibrations, persisted as TOML.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationStore {
    #[serde(default)]
    entries: BTreeMap<String, Calibration>,
}

impl CalibrationStore {
    pub fn get(&self, key: &CalibrationKey) -> Option<&Calibration> {
        self.entries.get(&key.to_storage_key())
    }

    pub fn set(&mut self, key: &CalibrationKey, value: Calibration) {
        self.entries.insert(key.to_storage_key(), value);
    }

    pub fn remove(&mut self, key: &CalibrationKey) -> bool {
        self.entries.remove(&key.to_storage_key()).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn to_toml(&self) -> Result<String, CalibrationError> {
        toml::to_string_pretty(self).map_err(|e| CalibrationError::Parse(e.to_string()))
    }

    pub fn from_toml(s: &str) -> Result<Self, CalibrationError> {
        toml::from_str(s).map_err(|e| CalibrationError::Parse(e.to_string()))
    }

    /// Load from disk, returning an empty store if the file does not exist yet.
    pub fn load(path: &Path) -> Result<Self, CalibrationError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CalibrationError::Io(e.to_string())),
        }
    }

    /// Write to disk, creating the parent directory if needed.
    ///
    /// Writes to a temporary file and renames, so an interrupted write cannot
    /// leave a truncated config behind.
    pub fn save(&self, path: &Path) -> Result<(), CalibrationError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CalibrationError::Io(e.to_string()))?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()?).map_err(|e| CalibrationError::Io(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| CalibrationError::Io(e.to_string()))
    }
}

/// Where the config lives: %APPDATA% on Windows, ~/.config elsewhere.
///
/// Deliberately not next to the executable, which may sit in Program Files
/// where a normal user cannot write.
pub fn config_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("DocumentsApp").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-08-07T19:00:00Z";

    #[test]
    fn perfect_print_needs_no_correction() {
        let c = Calibration::from_measurement(50.0, 50.0, 50.0, NOW).unwrap();
        assert_eq!(c.scale_x, 1.0);
        assert_eq!(c.scale_y, 1.0);
    }

    #[test]
    fn undersized_print_scales_up() {
        // Printed 49.5mm when 50mm was asked: output must grow by ~1%.
        let c = Calibration::from_measurement(50.0, 49.5, 50.0, NOW).unwrap();
        assert!(c.scale_x > 1.0, "expected scale up, got {}", c.scale_x);
        assert!((c.apply_x(50.0) - 50.505).abs() < 0.01);
        // The unmeasured axis stays untouched.
        assert_eq!(c.scale_y, 1.0);
    }

    #[test]
    fn axes_are_corrected_independently() {
        // Paper feed and head travel have different errors; that is the whole
        // reason scale_x and scale_y are separate.
        let c = Calibration::from_measurement(50.0, 49.5, 50.4, NOW).unwrap();
        assert!(c.scale_x > 1.0);
        assert!(c.scale_y < 1.0);
    }

    #[test]
    fn absurd_measurement_is_rejected() {
        // 30mm for a 50mm square: the user measured the wrong thing.
        let e = Calibration::from_measurement(50.0, 30.0, 50.0, NOW).unwrap_err();
        assert!(matches!(e, CalibrationError::ImplausibleScale { .. }));
    }

    #[test]
    fn zero_and_negative_measurements_rejected() {
        assert!(Calibration::from_measurement(50.0, 0.0, 50.0, NOW).is_err());
        assert!(Calibration::from_measurement(50.0, -1.0, 50.0, NOW).is_err());
        assert!(Calibration::from_measurement(0.0, 50.0, 50.0, NOW).is_err());
    }

    #[test]
    fn key_distinguishes_paper_and_borderless() {
        let a = CalibrationKey::new("P", 100.0, 150.0, false);
        let b = CalibrationKey::new("P", 100.0, 150.0, true);
        let c = CalibrationKey::new("P", 210.0, 297.0, false);
        assert_ne!(a.to_storage_key(), b.to_storage_key());
        assert_ne!(a.to_storage_key(), c.to_storage_key());
    }

    #[test]
    fn store_round_trips_through_toml() {
        let mut store = CalibrationStore::default();
        let key = CalibrationKey::new("Brother HL-L2402D Printer", 210.0, 297.0, false);
        let cal = Calibration::from_measurement(50.0, 49.8, 50.1, NOW).unwrap();
        store.set(&key, cal.clone());

        let toml = store.to_toml().unwrap();
        let back = CalibrationStore::from_toml(&toml).unwrap();
        let got = back.get(&key).expect("entry missing after round trip");
        assert!((got.scale_x - cal.scale_x).abs() < 1e-12);
        assert!((got.scale_y - cal.scale_y).abs() < 1e-12);
        assert_eq!(got.calibrated_at, NOW);
    }

    #[test]
    fn missing_entry_returns_none() {
        let store = CalibrationStore::default();
        let key = CalibrationKey::new("nonexistent", 100.0, 150.0, false);
        assert!(store.get(&key).is_none());
    }

    #[test]
    fn default_calibration_is_flagged_uncalibrated() {
        assert!(Calibration::default().is_uncalibrated());
        let c = Calibration::from_measurement(50.0, 50.0, 50.0, NOW).unwrap();
        assert!(!c.is_uncalibrated());
    }

    #[test]
    fn printer_names_with_separators_do_not_collide() {
        // The storage key is built with '|', so a printer name containing one
        // must not be able to impersonate another key.
        let a = CalibrationKey::new("A|100000x150000|false", 100.0, 150.0, false);
        let b = CalibrationKey::new("A", 100.0, 150.0, false);
        assert_ne!(a.to_storage_key(), b.to_storage_key());
    }
}
