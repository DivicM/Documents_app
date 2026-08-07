//! IPC commands. This is the only place the UI can reach the Rust side.
//!
//! Commands stay thin: validate input, call domain or platform, map errors to
//! something the UI can show. No layout or geometry logic lives here.

use domain::layout::{Alignment, LayoutConfig, SizeMm};
use domain::units;
use platform::calibration::{Calibration, CalibrationKey, CalibrationStore};
use platform::print::{PaperSize, PrintBackend, PrintJob};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use platform::WindowsPrintBackend;

/// Errors crossing the IPC boundary carry a message key plus values, so the UI
/// can localise them. Never a pre-formatted Croatian string.
#[derive(Debug, Serialize)]
pub struct UiError {
    pub key: String,
    pub params: serde_json::Value,
}

impl UiError {
    fn new(key: &str) -> Self {
        Self { key: key.into(), params: serde_json::json!({}) }
    }

    fn with(key: &str, params: serde_json::Value) -> Self {
        Self { key: key.into(), params }
    }
}

type CmdResult<T> = Result<T, UiError>;

#[cfg(windows)]
fn backend() -> WindowsPrintBackend {
    WindowsPrintBackend::new()
}

#[derive(Debug, Serialize)]
pub struct PrinterDto {
    pub name: String,
    pub driver: String,
    pub is_default: bool,
}

#[tauri::command]
pub fn list_printers() -> CmdResult<Vec<PrinterDto>> {
    #[cfg(windows)]
    {
        backend()
            .list_printers()
            .map(|ps| {
                ps.into_iter()
                    .map(|p| PrinterDto { name: p.name, driver: p.driver, is_default: p.is_default })
                    .collect()
            })
            .map_err(|e| UiError::with("error.printer.list_failed", serde_json::json!({ "detail": e.to_string() })))
    }
    #[cfg(not(windows))]
    {
        Err(UiError::new("error.platform.unsupported"))
    }
}

#[derive(Debug, Serialize)]
pub struct PrinterCapabilitiesDto {
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub margin_left_mm: f64,
    pub margin_top_mm: f64,
    pub margin_right_mm: f64,
    pub margin_bottom_mm: f64,
}

#[tauri::command]
pub fn printer_capabilities(
    printer: String,
    paper_width_mm: f64,
    paper_height_mm: f64,
) -> CmdResult<PrinterCapabilitiesDto> {
    #[cfg(windows)]
    {
        let b = backend();
        let dpi = b.device_dpi(&printer).map_err(|e| {
            UiError::with("error.printer.dpi_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;
        let paper = PaperSize { width_mm: paper_width_mm, height_mm: paper_height_mm };
        let m = b.hardware_margins_mm(&printer, paper).map_err(|e| {
            UiError::with("error.printer.margins_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;
        Ok(PrinterCapabilitiesDto {
            dpi_x: dpi.x,
            dpi_y: dpi.y,
            margin_left_mm: m.left_mm,
            margin_top_mm: m.top_mm,
            margin_right_mm: m.right_mm,
            margin_bottom_mm: m.bottom_mm,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (printer, paper_width_mm, paper_height_mm);
        Err(UiError::new("error.platform.unsupported"))
    }
}

#[derive(Debug, Deserialize)]
pub struct LayoutRequest {
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
    pub photo_width_mm: f64,
    pub photo_height_mm: f64,
    pub count: u32,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    pub align_top_left: bool,
}

#[derive(Debug, Serialize)]
pub struct PlacementDto {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub rotated: bool,
}

#[derive(Debug, Serialize)]
pub struct LayoutDto {
    pub placements: Vec<PlacementDto>,
    pub capacity_per_sheet: u32,
    pub sheets_needed: u32,
}

#[tauri::command]
pub fn solve_layout(req: LayoutRequest) -> CmdResult<LayoutDto> {
    let cfg = LayoutConfig {
        paper: SizeMm::new(req.paper_width_mm, req.paper_height_mm),
        photo: SizeMm::new(req.photo_width_mm, req.photo_height_mm),
        count: req.count,
        margin_mm: req.margin_mm,
        gutter_mm: req.gutter_mm,
        alignment: if req.align_top_left { Alignment::TopLeft } else { Alignment::Center },
    };

    let sheet = domain::layout::solve(&cfg).map_err(|e| match e {
        domain::layout::LayoutError::PhotoTooLarge => UiError::with(
            "error.layout.photo_too_large",
            serde_json::json!({
                "photoWidth": req.photo_width_mm,
                "photoHeight": req.photo_height_mm,
                "paperWidth": req.paper_width_mm,
                "paperHeight": req.paper_height_mm,
            }),
        ),
        domain::layout::LayoutError::NoUsableArea => UiError::new("error.layout.no_usable_area"),
        domain::layout::LayoutError::InvalidDimensions => {
            UiError::new("error.layout.invalid_dimensions")
        }
    })?;

    let rotated = sheet.orientation == domain::layout::Orientation::Landscape;
    Ok(LayoutDto {
        placements: sheet
            .placements
            .iter()
            .map(|p| PlacementDto {
                x_mm: p.x_mm,
                y_mm: p.y_mm,
                width_mm: p.size.width,
                height_mm: p.size.height,
                rotated,
            })
            .collect(),
        capacity_per_sheet: sheet.capacity_per_sheet,
        sheets_needed: sheet.sheets_needed,
    })
}

#[derive(Debug, Serialize)]
pub struct ResolutionCheckDto {
    pub required_px_w: u32,
    pub required_px_h: u32,
    pub source_px_w: u32,
    pub source_px_h: u32,
    pub would_upscale: bool,
    pub max_lossless_dpi: f64,
}

/// Report whether a source image has the pixels for a target size at a DPI.
///
/// The UI shows these numbers rather than silently interpolating, per the rule
/// that upscaling must never happen without the user saying so.
#[tauri::command]
pub fn check_resolution(
    source_px_w: u32,
    source_px_h: u32,
    target_width_mm: f64,
    target_height_mm: f64,
    dpi: f64,
) -> CmdResult<ResolutionCheckDto> {
    if dpi <= 0.0 || target_width_mm <= 0.0 || target_height_mm <= 0.0 {
        return Err(UiError::new("error.layout.invalid_dimensions"));
    }

    let required_w = units::mm_to_px(target_width_mm, dpi);
    let required_h = units::mm_to_px(target_height_mm, dpi);

    Ok(ResolutionCheckDto {
        required_px_w: required_w,
        required_px_h: required_h,
        source_px_w,
        source_px_h,
        would_upscale: required_w > source_px_w || required_h > source_px_h,
        max_lossless_dpi: units::max_lossless_dpi(source_px_w, target_width_mm)
            .min(units::max_lossless_dpi(source_px_h, target_height_mm)),
    })
}

fn config_path() -> CmdResult<std::path::PathBuf> {
    platform::calibration::config_path().ok_or_else(|| UiError::new("error.config.no_path"))
}

#[derive(Debug, Serialize)]
pub struct CalibrationDto {
    pub scale_x: f64,
    pub scale_y: f64,
    pub offset_x_mm: f64,
    pub offset_y_mm: f64,
    pub calibrated_at: String,
}

#[tauri::command]
pub fn get_calibration(
    printer: String,
    paper_width_mm: f64,
    paper_height_mm: f64,
    borderless: bool,
) -> CmdResult<Option<CalibrationDto>> {
    let store = CalibrationStore::load(&config_path()?).map_err(|e| {
        UiError::with("error.config.load_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;
    let key = CalibrationKey::new(&printer, paper_width_mm, paper_height_mm, borderless);
    Ok(store.get(&key).map(|c| CalibrationDto {
        scale_x: c.scale_x,
        scale_y: c.scale_y,
        offset_x_mm: c.offset_x_mm,
        offset_y_mm: c.offset_y_mm,
        calibrated_at: c.calibrated_at.clone(),
    }))
}

/// Store a calibration derived from a measured test square.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn save_calibration(
    printer: String,
    paper_width_mm: f64,
    paper_height_mm: f64,
    borderless: bool,
    nominal_mm: f64,
    measured_x_mm: f64,
    measured_y_mm: f64,
    now_rfc3339: String,
) -> CmdResult<CalibrationDto> {
    let cal = Calibration::from_measurement(nominal_mm, measured_x_mm, measured_y_mm, &now_rfc3339)
        .map_err(|e| match e {
            platform::CalibrationError::ImplausibleScale { scale_x, scale_y } => UiError::with(
                "error.calibration.implausible",
                serde_json::json!({ "scaleX": scale_x, "scaleY": scale_y }),
            ),
            other => UiError::with(
                "error.calibration.invalid",
                serde_json::json!({ "detail": other.to_string() }),
            ),
        })?;

    let path = config_path()?;
    let mut store = CalibrationStore::load(&path).map_err(|e| {
        UiError::with("error.config.load_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;
    let key = CalibrationKey::new(&printer, paper_width_mm, paper_height_mm, borderless);
    store.set(&key, cal.clone());
    store.save(&path).map_err(|e| {
        UiError::with("error.config.save_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;

    Ok(CalibrationDto {
        scale_x: cal.scale_x,
        scale_y: cal.scale_y,
        offset_x_mm: cal.offset_x_mm,
        offset_y_mm: cal.offset_y_mm,
        calibrated_at: cal.calibrated_at,
    })
}

/// Nominal edge length of the calibration square, in millimetres.
pub const CALIBRATION_SQUARE_MM: f64 = 50.0;

/// A4 is assumed for the calibration print: it is what a plain printer is most
/// likely to have loaded.
const CALIBRATION_PAPER: (f64, f64) = (210.0, 297.0);

/// Load the stored calibration for a printer, or identity if none exists.
///
/// Deliberately never fails: a missing or unreadable config must not stop
/// someone printing, it just means no correction is applied.
#[cfg(windows)]
fn calibration_for(
    printer: &str,
    paper_width_mm: f64,
    paper_height_mm: f64,
    borderless: bool,
) -> domain::render::CalibrationScale {
    use domain::render::CalibrationScale;

    let Some(path) = platform::calibration::config_path() else {
        return CalibrationScale::IDENTITY;
    };
    let Ok(store) = CalibrationStore::load(&path) else {
        return CalibrationScale::IDENTITY;
    };
    let key = CalibrationKey::new(printer, paper_width_mm, paper_height_mm, borderless);
    match store.get(&key) {
        Some(c) => CalibrationScale {
            scale_x: c.scale_x,
            scale_y: c.scale_y,
            offset_x_mm: c.offset_x_mm,
            offset_y_mm: c.offset_y_mm,
        },
        None => CalibrationScale::IDENTITY,
    }
}

/// Print the 50x50mm calibration square.
///
/// `apply_calibration` is false for the first test print, which must show the
/// printer's raw error, and true for a verification print afterwards.
#[tauri::command]
pub fn print_calibration_square(printer: String, apply_calibration: bool) -> CmdResult<u32> {
    #[cfg(windows)]
    {
        use domain::layout::{Orientation, Placement, Sheet};
        use domain::render::{render_sheet, CalibrationScale, PrintableOrigin, RenderParams};

        let b = backend();
        let dpi = b.device_dpi(&printer).map_err(|e| {
            UiError::with("error.printer.dpi_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;

        let paper = SizeMm::new(CALIBRATION_PAPER.0, CALIBRATION_PAPER.1);
        let m = b
            .hardware_margins_mm(&printer, PaperSize { width_mm: paper.width, height_mm: paper.height })
            .unwrap_or(platform::print::Margins::ZERO);

        let square = Placement {
            x_mm: 20.0,
            y_mm: 20.0,
            size: SizeMm::new(CALIBRATION_SQUARE_MM, CALIBRATION_SQUARE_MM),
            orientation: Orientation::Portrait,
        };
        let sheet = Sheet {
            placements: vec![square],
            orientation: Orientation::Portrait,
            capacity_per_sheet: 1,
            sheets_needed: 1,
        };

        let printable = SizeMm::new(
            paper.width - m.left_mm - m.right_mm,
            paper.height - m.top_mm - m.bottom_mm,
        );

        let mut params = RenderParams::new(dpi.x as f64, dpi.y as f64);
        params.origin = PrintableOrigin { left_mm: m.left_mm, top_mm: m.top_mm };
        params.calibration = if apply_calibration {
            calibration_for(&printer, paper.width, paper.height, false)
        } else {
            CalibrationScale::IDENTITY
        };

        let raster = render_sheet(&sheet, printable, &params);

        let job = PrintJob {
            printer: printer.clone(),
            pixels: raster.pixels,
            width_px: raster.width_px,
            height_px: raster.height_px,
            document_name: "Kalibracija 50x50mm".into(),
        };

        b.print_raster(&job).map(|id| id.0).map_err(|e| {
            UiError::with("error.print.failed", serde_json::json!({ "detail": e.to_string() }))
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (printer, apply_calibration);
        Err(UiError::new("error.platform.unsupported"))
    }
}

/// Print the laid-out sheet, with the stored calibration applied.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn print_sheet(
    printer: String,
    paper_width_mm: f64,
    paper_height_mm: f64,
    photo_width_mm: f64,
    photo_height_mm: f64,
    count: u32,
    margin_mm: f64,
    gutter_mm: f64,
    align_top_left: bool,
) -> CmdResult<u32> {
    #[cfg(windows)]
    {
        use domain::render::{render_sheet, PrintableOrigin, RenderParams};

        let cfg = LayoutConfig {
            paper: SizeMm::new(paper_width_mm, paper_height_mm),
            photo: SizeMm::new(photo_width_mm, photo_height_mm),
            count,
            margin_mm,
            gutter_mm,
            alignment: if align_top_left { Alignment::TopLeft } else { Alignment::Center },
        };
        let sheet = domain::layout::solve(&cfg)
            .map_err(|_| UiError::new("error.layout.invalid_dimensions"))?;

        if sheet.placements.is_empty() {
            return Err(UiError::new("error.print.nothing_to_print"));
        }

        let b = backend();
        let dpi = b.device_dpi(&printer).map_err(|e| {
            UiError::with("error.printer.dpi_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;
        let m = b
            .hardware_margins_mm(
                &printer,
                PaperSize { width_mm: paper_width_mm, height_mm: paper_height_mm },
            )
            .unwrap_or(platform::print::Margins::ZERO);

        let printable = SizeMm::new(
            paper_width_mm - m.left_mm - m.right_mm,
            paper_height_mm - m.top_mm - m.bottom_mm,
        );

        let mut params = RenderParams::new(dpi.x as f64, dpi.y as f64);
        params.origin = PrintableOrigin { left_mm: m.left_mm, top_mm: m.top_mm };
        params.calibration = calibration_for(&printer, paper_width_mm, paper_height_mm, false);

        let raster = render_sheet(&sheet, printable, &params);

        let job = PrintJob {
            printer: printer.clone(),
            pixels: raster.pixels,
            width_px: raster.width_px,
            height_px: raster.height_px,
            document_name: "Fotografije za dokumente".into(),
        };

        b.print_raster(&job).map(|id| id.0).map_err(|e| {
            UiError::with("error.print.failed", serde_json::json!({ "detail": e.to_string() }))
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (
            printer,
            paper_width_mm,
            paper_height_mm,
            photo_width_mm,
            photo_height_mm,
            count,
            margin_mm,
            gutter_mm,
            align_top_left,
        );
        Err(UiError::new("error.platform.unsupported"))
    }
}
