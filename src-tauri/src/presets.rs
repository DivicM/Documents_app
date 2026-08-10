//! Preset commands: save, load, list and delete named setting sets.
//!
//! Presets share `config.toml` with the calibrations, so every write goes
//! through `Config`, which carries both. Reading one and writing only that half
//! would drop the other.

use platform::presets::{Config, Preset};
use serde::{Deserialize, Serialize};

use crate::commands::UiError;

/// A preset as it crosses the IPC boundary.
///
/// A separate type from `platform::Preset` on purpose: the wire shape is a
/// contract with the frontend, and letting a storage struct double as one means
/// a rename in storage silently breaks the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetDto {
    pub spec_id: String,
    pub photo_width_mm: f64,
    pub photo_height_mm: f64,
    pub head_height_mm: f64,
    pub paper_id: String,
    pub count: u32,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    pub align_top_left: bool,
    pub cut_marks: bool,
    pub replace_background: bool,
    pub background_rgb: [u8; 3],
    pub exposure_ev: f64,
    pub contrast: f64,
    pub temperature: f64,
    pub tint: f64,
}

impl From<&Preset> for PresetDto {
    fn from(p: &Preset) -> Self {
        Self {
            spec_id: p.spec_id.clone(),
            photo_width_mm: p.photo_width_mm,
            photo_height_mm: p.photo_height_mm,
            head_height_mm: p.head_height_mm,
            paper_id: p.paper_id.clone(),
            count: p.count,
            margin_mm: p.margin_mm,
            gutter_mm: p.gutter_mm,
            align_top_left: p.align_top_left,
            cut_marks: p.cut_marks,
            replace_background: p.replace_background,
            background_rgb: p.background_rgb,
            exposure_ev: p.exposure_ev,
            contrast: p.contrast,
            temperature: p.temperature,
            tint: p.tint,
        }
    }
}

impl PresetDto {
    fn into_preset(self, saved_at: String) -> Preset {
        Preset {
            spec_id: self.spec_id,
            photo_width_mm: self.photo_width_mm,
            photo_height_mm: self.photo_height_mm,
            head_height_mm: self.head_height_mm,
            paper_id: self.paper_id,
            count: self.count,
            margin_mm: self.margin_mm,
            gutter_mm: self.gutter_mm,
            align_top_left: self.align_top_left,
            cut_marks: self.cut_marks,
            replace_background: self.replace_background,
            background_rgb: self.background_rgb,
            exposure_ev: self.exposure_ev,
            contrast: self.contrast,
            temperature: self.temperature,
            tint: self.tint,
            saved_at,
        }
    }
}

/// One entry in the preset list.
#[derive(Debug, Serialize)]
pub struct PresetSummary {
    pub name: String,
    pub saved_at: String,
}

fn config_path() -> Result<std::path::PathBuf, UiError> {
    platform::calibration::config_path().ok_or_else(|| UiError::new("error.config.no_path"))
}

fn load_config() -> Result<Config, UiError> {
    Config::load(&config_path()?).map_err(|e| {
        UiError::with("error.config.load_failed", serde_json::json!({ "detail": e.to_string() }))
    })
}

/// Sheet settings as they cross the IPC boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetSettingsDto {
    pub paper_id: String,
    pub count: u32,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    pub align_top_left: bool,
    pub cut_marks: bool,
    pub quarter_turn: bool,
    pub turn_photo: bool,
    pub printer: String,
}

impl From<&platform::presets::SheetSettings> for SheetSettingsDto {
    fn from(s: &platform::presets::SheetSettings) -> Self {
        Self {
            paper_id: s.paper_id.clone(),
            count: s.count,
            margin_mm: s.margin_mm,
            gutter_mm: s.gutter_mm,
            align_top_left: s.align_top_left,
            cut_marks: s.cut_marks,
            quarter_turn: s.quarter_turn,
            turn_photo: s.turn_photo,
            printer: s.printer.clone(),
        }
    }
}

/// Read the persisted sheet settings, or defaults if none are stored yet.
#[tauri::command]
pub fn get_sheet_settings() -> Result<SheetSettingsDto, UiError> {
    Ok(SheetSettingsDto::from(&load_config()?.sheet))
}

/// Persist the sheet settings, leaving calibrations and presets untouched.
#[tauri::command]
pub fn save_sheet_settings(settings: SheetSettingsDto) -> Result<(), UiError> {
    let mut cfg = load_config()?;
    cfg.sheet = platform::presets::SheetSettings {
        paper_id: settings.paper_id,
        count: settings.count,
        margin_mm: settings.margin_mm,
        gutter_mm: settings.gutter_mm,
        align_top_left: settings.align_top_left,
        cut_marks: settings.cut_marks,
        quarter_turn: settings.quarter_turn,
        turn_photo: settings.turn_photo,
        printer: settings.printer,
    };
    cfg.save(&config_path()?).map_err(|e| {
        UiError::with("error.config.save_failed", serde_json::json!({ "detail": e.to_string() }))
    })
}

#[tauri::command]
pub fn list_presets() -> Result<Vec<PresetSummary>, UiError> {
    let cfg = load_config()?;
    let store = cfg.store();
    Ok(store
        .names()
        .into_iter()
        .map(|name| {
            let saved_at = store.get(&name).map(|p| p.saved_at.clone()).unwrap_or_default();
            PresetSummary { name, saved_at }
        })
        .collect())
}

#[tauri::command]
pub fn save_preset(
    name: String,
    preset: PresetDto,
    now_rfc3339: String,
) -> Result<Vec<PresetSummary>, UiError> {
    let mut cfg = load_config()?;
    let mut store = cfg.store();

    store.set(&name, preset.into_preset(now_rfc3339)).map_err(|e| match e {
        platform::PresetError::EmptyName => UiError::new("error.preset.empty_name"),
        platform::PresetError::NameTooLong { max } => {
            UiError::with("error.preset.name_too_long", serde_json::json!({ "max": max }))
        }
        platform::PresetError::NotFound => UiError::new("error.preset.not_found"),
    })?;

    cfg.set_store(store);
    cfg.save(&config_path()?).map_err(|e| {
        UiError::with("error.config.save_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;

    list_presets()
}

#[tauri::command]
pub fn load_preset(name: String) -> Result<PresetDto, UiError> {
    let cfg = load_config()?;
    cfg.store()
        .get(name.trim())
        .map(PresetDto::from)
        .ok_or_else(|| UiError::with("error.preset.not_found", serde_json::json!({ "name": name })))
}

#[tauri::command]
pub fn delete_preset(name: String) -> Result<Vec<PresetSummary>, UiError> {
    let mut cfg = load_config()?;
    let mut store = cfg.store();
    if !store.remove(&name) {
        return Err(UiError::with(
            "error.preset.not_found",
            serde_json::json!({ "name": name }),
        ));
    }
    cfg.set_store(store);
    cfg.save(&config_path()?).map_err(|e| {
        UiError::with("error.config.save_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;
    list_presets()
}
