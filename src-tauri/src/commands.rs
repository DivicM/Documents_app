//! IPC commands. This is the only place the UI can reach the Rust side.
//!
//! Commands stay thin: validate input, call domain or platform, map errors to
//! something the UI can show. No layout or geometry logic lives here.

use domain::layout::{Alignment, LayoutConfig, SizeMm};
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
    pub fn new(key: &str) -> Self {
        Self { key: key.into(), params: serde_json::json!({}) }
    }

    pub fn with(key: &str, params: serde_json::Value) -> Self {
        Self { key: key.into(), params }
    }
}

type CmdResult<T> = Result<T, UiError>;

/// Read a `u32` header, which is how image dimensions travel alongside a raw
/// body.
///
/// Commands taking a whole photo receive it as raw bytes rather than a JSON
/// number array: a 2000x1333 image is 10.7MB, which as JSON becomes 32MB of
/// text and costs around two seconds to encode and parse.
pub fn header_u32(req: &tauri::ipc::Request<'_>, name: &str) -> CmdResult<u32> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| UiError::new("error.image.empty"))
}

/// Borrow the raw body of a request, rejecting a JSON one.
pub fn raw_body<'a>(req: &'a tauri::ipc::Request<'_>) -> CmdResult<&'a [u8]> {
    match req.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => Ok(bytes),
        tauri::ipc::InvokeBody::Json(_) => Err(UiError::new("error.image.decode_failed")),
    }
}

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
    /// The paper the driver is set to, which is not necessarily the one the
    /// user picked in the app. A dye-sublimation photo printer is configured
    /// for one size and cannot be told otherwise from here.
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
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
        // Falls back to the requested size if the driver will not say, which
        // is no worse than the old behaviour of always assuming it.
        let dp = b.device_paper(&printer).ok();

        Ok(PrinterCapabilitiesDto {
            dpi_x: dpi.x,
            dpi_y: dpi.y,
            margin_left_mm: m.left_mm,
            margin_top_mm: m.top_mm,
            margin_right_mm: m.right_mm,
            margin_bottom_mm: m.bottom_mm,
            paper_width_mm: dp.map(|d| d.physical_width_mm).unwrap_or(paper_width_mm),
            paper_height_mm: dp.map(|d| d.physical_height_mm).unwrap_or(paper_height_mm),
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
    /// Turn the frame on its side, as [`PrintSheetRequest::quarter_turn`] does.
    /// The preview must solve the same layout the printer will.
    #[serde(default)]
    pub quarter_turn: bool,
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
    let (photo_w, photo_h) = if req.quarter_turn {
        (req.photo_height_mm, req.photo_width_mm)
    } else {
        (req.photo_width_mm, req.photo_height_mm)
    };

    let cfg = LayoutConfig {
        paper: SizeMm::new(req.paper_width_mm, req.paper_height_mm),
        photo: SizeMm::new(photo_w, photo_h),
        count: req.count,
        margin_mm: req.margin_mm,
        gutter_mm: req.gutter_mm,
        alignment: if req.align_top_left { Alignment::TopLeft } else { Alignment::Center },
        // Always locked: the requested shape is the one that prints. Letting
        // the solver rotate for density is what laid every photo on its side
        // to fit one more on the sheet.
        lock_orientation: true,
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

/// One photo size and how many copies of it belong on the mixed sheet.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PhotoGroupRequest {
    pub width_mm: f64,
    pub height_mm: f64,
    pub count: u32,
}

#[derive(Debug, Deserialize)]
pub struct MixedLayoutRequest {
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
    pub groups: Vec<PhotoGroupRequest>,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    #[serde(default)]
    pub quarter_turn: bool,
}

#[derive(Debug, Serialize)]
pub struct GroupedPlacementDto {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub rotated: bool,
    /// Index into the requested groups, so the UI can colour or label them.
    pub group: usize,
}

#[derive(Debug, Serialize)]
pub struct MixedLayoutDto {
    pub placements: Vec<GroupedPlacementDto>,
    /// Copies of each group that did not fit. Reported rather than dropped.
    pub unplaced: Vec<u32>,
}

/// Convert the requested groups, optionally turning each frame on its side.
///
/// Turning swaps width and height before the solver runs, which is what makes a
/// 35x45 frame become 45x35 with the face still upright inside it.
fn to_groups_turned(
    groups: &[PhotoGroupRequest],
    quarter_turn: bool,
) -> Vec<domain::layout::PhotoGroup> {
    groups
        .iter()
        .map(|g| domain::layout::PhotoGroup {
            size: if quarter_turn {
                SizeMm::new(g.height_mm, g.width_mm)
            } else {
                SizeMm::new(g.width_mm, g.height_mm)
            },
            count: g.count,
        })
        .collect()
}

fn map_layout_error(e: domain::layout::LayoutError) -> UiError {
    match e {
        domain::layout::LayoutError::PhotoTooLarge => UiError::new("error.layout.photo_too_large"),
        domain::layout::LayoutError::NoUsableArea => UiError::new("error.layout.no_usable_area"),
        domain::layout::LayoutError::InvalidDimensions => {
            UiError::new("error.layout.invalid_dimensions")
        }
    }
}

/// Arrange several photo sizes on one sheet (§7).
#[tauri::command]
pub fn solve_mixed_layout(req: MixedLayoutRequest) -> CmdResult<MixedLayoutDto> {
    let groups = to_groups_turned(&req.groups, req.quarter_turn);
    let sheet = domain::layout::solve_mixed_with(
        SizeMm::new(req.paper_width_mm, req.paper_height_mm),
        &groups,
        req.margin_mm,
        req.gutter_mm,
        // Locked, as in the single-size path: no rotating for density.
        true,
    )
    .map_err(map_layout_error)?;

    Ok(MixedLayoutDto {
        placements: sheet
            .placements
            .iter()
            .map(|g| GroupedPlacementDto {
                x_mm: g.placement.x_mm,
                y_mm: g.placement.y_mm,
                width_mm: g.placement.size.width,
                height_mm: g.placement.size.height,
                rotated: g.placement.orientation == domain::layout::Orientation::Landscape,
                group: g.group,
            })
            .collect(),
        unplaced: sheet.unplaced,
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
/// The paper the driver is set to, falling back to what the caller asked for.
///
/// The driver wins: a dye-sublimation photo printer is configured for one paper
/// size and ignores anything else sent to it, so laying out for a different
/// size is what puts photographs off the edge of the sheet. A Citizen CY-02
/// reports 156x105mm, which is both a different size and a different
/// orientation from the 100x150mm a user would naturally pick.
#[cfg(windows)]
fn device_paper_or(
    backend: &WindowsPrintBackend,
    printer: &str,
    fallback_w: f64,
    fallback_h: f64,
) -> (f64, f64) {
    match backend.device_paper(printer) {
        // Guard against a driver reporting nonsense rather than failing.
        Ok(dp) if dp.physical_width_mm > 1.0 && dp.physical_height_mm > 1.0 => {
            (dp.physical_width_mm, dp.physical_height_mm)
        }
        _ => (fallback_w, fallback_h),
    }
}

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

/// The photo to place on the sheet: everything except the pixels themselves.
///
/// Pixels come from the webview's canvas, which has already decoded the file,
/// and travel in the raw request body rather than in this struct. A 10.7MB
/// photo as a JSON number array becomes 32MB of text and costs about two
/// seconds to encode and parse.
#[derive(Debug, Deserialize)]
pub struct PhotoPayload {
    pub width: u32,
    pub height: u32,
    pub crop_x: f64,
    pub crop_y: f64,
    pub crop_width: f64,
    pub crop_height: f64,
    /// Head tilt to straighten, in degrees. Zero leaves the crop untouched.
    #[serde(default)]
    pub rotation_deg: f64,
    /// Background replacement. Omitted to print the photo as taken.
    #[serde(default)]
    pub background: Option<BackgroundPayload>,
    /// Exposure and white balance. Omitted leaves the photo as taken.
    #[serde(default)]
    pub adjustments: Option<domain::adjust::Adjustments>,
}

/// Mask dimensions and the colour to put behind the subject. The mask bytes
/// follow the photo in the raw body.
#[derive(Debug, Deserialize)]
pub struct BackgroundPayload {
    pub mask_width: u32,
    pub mask_height: u32,
    pub colour: [u8; 3],
}

/// The pixel buffers that accompany a print request, split out of the raw body.
///
/// The photo is followed by the mask with nothing between them: both lengths
/// are known from the JSON parameters, so no framing is needed.
pub struct PrintPixels<'a> {
    pub rgba: &'a [u8],
    pub mask: &'a [u8],
}

/// Parse a print request body: a length-prefixed JSON header, then the pixels.
///
/// `invoke` carries either JSON arguments or a raw body, never both, so the
/// parameters lead the body instead of being a separate argument. Returns the
/// parsed parameters and the remaining bytes.
fn parse_print_request<'a, T: serde::de::DeserializeOwned>(
    request: &'a tauri::ipc::Request<'_>,
) -> CmdResult<(T, &'a [u8])> {
    let body = raw_body(request)?;
    if body.len() < 4 {
        return Err(UiError::new("error.image.decode_failed"));
    }

    let json_len = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
    let rest = body.get(4..).ok_or_else(|| UiError::new("error.image.decode_failed"))?;
    let json = rest.get(..json_len).ok_or_else(|| UiError::new("error.image.decode_failed"))?;

    let req: T = serde_json::from_slice(json).map_err(|e| {
        UiError::with("error.image.decode_failed", serde_json::json!({ "detail": e.to_string() }))
    })?;
    Ok((req, &rest[json_len..]))
}

/// Split the pixel section into the photo and, if one is expected, the mask.
///
/// Validates both lengths up front. Getting this wrong would index out of
/// bounds deep inside the renderer, so it fails here with a clear message.
///
/// The arithmetic is checked because the dimensions come off the wire: on a
/// 32-bit target a large width times height overflows, and a wrapped product
/// could match the body length and admit a buffer the renderer would then read
/// past. An overflow is simply a length no body can have, so it takes the same
/// mismatch path as any other wrong size.
fn split_print_body<'a>(body: &'a [u8], photo: &PhotoPayload) -> CmdResult<PrintPixels<'a>> {
    let size_mismatch = |expected: serde_json::Value| {
        UiError::with(
            "error.image.size_mismatch",
            serde_json::json!({ "expected": expected, "got": body.len() }),
        )
    };

    let photo_len = (photo.width as usize)
        .checked_mul(photo.height as usize)
        .and_then(|px| px.checked_mul(4));
    let mask_len = match photo.background.as_ref() {
        Some(b) => (b.mask_width as usize).checked_mul(b.mask_height as usize),
        None => Some(0),
    };

    let expected = photo_len.zip(mask_len).and_then(|(p, m)| p.checked_add(m));
    let (Some(photo_len), Some(expected)) = (photo_len, expected) else {
        // Too large to be a real buffer, so report it as the mismatch it is
        // rather than naming a number that overflowed.
        return Err(size_mismatch(serde_json::Value::Null));
    };

    if body.len() != expected {
        return Err(size_mismatch(expected.into()));
    }

    Ok(PrintPixels { rgba: &body[..photo_len], mask: &body[photo_len..] })
}

#[derive(Debug, Deserialize)]
pub struct PrintSheetRequest {
    pub printer: String,
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
    pub photo_width_mm: f64,
    pub photo_height_mm: f64,
    pub count: u32,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    pub align_top_left: bool,
    /// Turn the photo frame on its side: a 35x45 print becomes 45x35.
    ///
    /// Applied by swapping the requested width and height before the layout is
    /// solved, so the solver, the renderer and the crop all agree.
    #[serde(default)]
    pub quarter_turn: bool,
    /// Turn the picture inside its frame. Independent of `quarter_turn`, which
    /// only reshapes the rectangle.
    #[serde(default)]
    pub turn_photo: bool,
    /// Print faint guides showing where each photo ends, for cutting by hand.
    #[serde(default)]
    pub cut_marks: bool,
    /// Print a guide along the bottom edge of each photo only.
    #[serde(default)]
    pub bottom_mark: bool,
    /// Omitted to print the layout as plain rectangles, which is useful for
    /// checking geometry without using up photo paper.
    pub photo: Option<PhotoPayload>,
}

#[derive(Debug, Deserialize)]
pub struct PrintMixedRequest {
    pub printer: String,
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
    pub groups: Vec<PhotoGroupRequest>,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    /// Turn each frame on its side.
    #[serde(default)]
    pub quarter_turn: bool,
    /// Turn the picture inside its frame.
    #[serde(default)]
    pub turn_photo: bool,
    /// Print faint guides showing where each photo ends.
    #[serde(default)]
    pub cut_marks: bool,
    /// Print a guide along the bottom edge of each photo only.
    #[serde(default)]
    pub bottom_mark: bool,
    pub photo: Option<PhotoPayload>,
}

/// Apply tone and background to the source pixels, in the order the preview uses.
///
/// Shared by both print paths so a change here cannot make the mixed sheet
/// composite differently from the ordinary one.
#[cfg(windows)]
fn prepared_pixels(p: &PhotoPayload, px: &PrintPixels<'_>) -> CmdResult<Vec<u8>> {
    // Order matters and must match the preview: tone first, then background.
    // Adjusting after replacement would shift the background colour the user
    // picked.
    let mut pixels = px.rgba.to_vec();
    if let Some(adj) = &p.adjustments {
        domain::adjust::apply_adjustments(&mut pixels, adj);
    }
    if let Some(bg) = &p.background {
        let mask = domain::mask::AlphaMask::new(bg.mask_width, bg.mask_height, px.mask.to_vec())
            .ok_or_else(|| UiError::new("error.mask.size_mismatch"))?;
        domain::mask::composite_background(&mut pixels, p.width, p.height, &mask, bg.colour);
    }
    Ok(pixels)
}

/// Print a sheet holding several photo sizes at once (§7).
///
/// Parameters arrive as JSON in the `req` argument; the photo and mask pixels
/// arrive as the raw request body, for the reason given on [`PhotoPayload`].
#[tauri::command]
pub fn print_mixed_sheet(request: tauri::ipc::Request<'_>) -> CmdResult<u32> {
    let (req, pixels) = parse_print_request::<PrintMixedRequest>(&request)?;
    #[cfg(windows)]
    {
        use domain::layout::Placement;
        use domain::render::{
            render_mixed_sheet, render_sheet, PhotoSource, PrintableOrigin, RenderParams,
        };
        use domain::resample::ImageRef;

        let b = backend();
        let dpi = b.device_dpi(&req.printer).map_err(|e| {
            UiError::with("error.printer.dpi_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;

        // Resolve the paper before laying anything out.
        let (paper_w, paper_h) = device_paper_or(&b, &req.printer, req.paper_width_mm, req.paper_height_mm);
        let paper = SizeMm::new(paper_w, paper_h);

        let groups = to_groups_turned(&req.groups, req.quarter_turn);
        let mixed = domain::layout::solve_mixed_with(
            paper,
            &groups,
            req.margin_mm,
            req.gutter_mm,
            // Locked, as in the single-size path: no rotating for density.
            true,
        )
        .map_err(map_layout_error)?;

        if mixed.placements.is_empty() {
            return Err(UiError::new("error.print.nothing_to_print"));
        }

        let m = b
            .hardware_margins_mm(
                &req.printer,
                PaperSize { width_mm: paper_w, height_mm: paper_h },
            )
            .unwrap_or(platform::print::Margins::ZERO);

        let printable = SizeMm::new(
            paper_w - m.left_mm - m.right_mm,
            paper_h - m.top_mm - m.bottom_mm,
        );

        let mut params = RenderParams::new(dpi.x as f64, dpi.y as f64);
        params.origin = PrintableOrigin { left_mm: m.left_mm, top_mm: m.top_mm };
        params.calibration = calibration_for(&req.printer, paper_w, paper_h, false);
        params.turn_photo = req.turn_photo;
        params.cut_marks = req.cut_marks;
        params.bottom_mark = req.bottom_mark;

        let placements: Vec<Placement> =
            mixed.placements.iter().map(|g| g.placement).collect();

        let raster = match &req.photo {
            Some(p) => {
                let split = split_print_body(pixels, p)?;
                let pixels = prepared_pixels(p, &split)?;
                let image = ImageRef::new(&pixels, p.width, p.height)
                    .ok_or_else(|| UiError::new("error.image.decode_failed"))?;
                let source = PhotoSource {
                    image,
                    crop_x: p.crop_x,
                    crop_y: p.crop_y,
                    crop_width: p.crop_width,
                    crop_height: p.crop_height,
                    rotation_deg: p.rotation_deg,
                };
                render_mixed_sheet(&placements, printable, &params, &source)
            }
            None => {
                // Plain rectangles, for checking geometry without photo paper.
                let sheet = domain::layout::Sheet {
                    placements,
                    orientation: domain::layout::Orientation::Portrait,
                    capacity_per_sheet: mixed.placements.len() as u32,
                    sheets_needed: 1,
                };
                render_sheet(&sheet, printable, &params)
            }
        };

        let job = PrintJob {
            printer: req.printer.clone(),
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
        let _ = (req, pixels);
        Err(UiError::new("error.platform.unsupported"))
    }
}

/// Print the laid-out sheet, with the stored calibration applied.
///
/// Parameters arrive as JSON in `req`; the photo and mask pixels arrive as the
/// raw request body, for the reason given on [`PhotoPayload`].
#[tauri::command]
pub fn print_sheet(request: tauri::ipc::Request<'_>) -> CmdResult<u32> {
    let (req, pixels) = parse_print_request::<PrintSheetRequest>(&request)?;
    #[cfg(windows)]
    {
        use domain::render::{
            render_sheet, render_sheet_with_photo, PhotoSource, PrintableOrigin, RenderParams,
        };
        use domain::resample::ImageRef;

        // Turning the frame is a swap of the requested print size, done before
        // anything else sees it. The crop the frontend sent already has the
        // photo's aspect ratio, so the renderer fills the turned frame with the
        // same upright pixels.
        let (photo_w, photo_h) = if req.quarter_turn {
            (req.photo_height_mm, req.photo_width_mm)
        } else {
            (req.photo_width_mm, req.photo_height_mm)
        };

        let b = backend();
        let dpi = b.device_dpi(&req.printer).map_err(|e| {
            UiError::with("error.printer.dpi_failed", serde_json::json!({ "detail": e.to_string() }))
        })?;

        // Resolve the paper before laying anything out.
        let (paper_w, paper_h) =
            device_paper_or(&b, &req.printer, req.paper_width_mm, req.paper_height_mm);

        let cfg = LayoutConfig {
            paper: SizeMm::new(paper_w, paper_h),
            photo: SizeMm::new(photo_w, photo_h),
            count: req.count,
            margin_mm: req.margin_mm,
            gutter_mm: req.gutter_mm,
            alignment: if req.align_top_left { Alignment::TopLeft } else { Alignment::Center },
            // Always locked, as in solve_layout: the requested shape is the
            // one that prints, whether or not the frame was turned.
            lock_orientation: true,
        };
        let sheet = domain::layout::solve(&cfg)
            .map_err(|_| UiError::new("error.layout.invalid_dimensions"))?;

        if sheet.placements.is_empty() {
            return Err(UiError::new("error.print.nothing_to_print"));
        }

        let m = b
            .hardware_margins_mm(
                &req.printer,
                PaperSize { width_mm: paper_w, height_mm: paper_h },
            )
            .unwrap_or(platform::print::Margins::ZERO);

        let printable = SizeMm::new(
            paper_w - m.left_mm - m.right_mm,
            paper_h - m.top_mm - m.bottom_mm,
        );

        let mut params = RenderParams::new(dpi.x as f64, dpi.y as f64);
        params.origin = PrintableOrigin { left_mm: m.left_mm, top_mm: m.top_mm };
        params.calibration = calibration_for(&req.printer, paper_w, paper_h, false);
        params.turn_photo = req.turn_photo;
        params.cut_marks = req.cut_marks;
        params.bottom_mark = req.bottom_mark;

        let raster = match &req.photo {
            Some(p) => {
                let split = split_print_body(pixels, p)?;
                let pixels = prepared_pixels(p, &split)?;
                let image = ImageRef::new(&pixels, p.width, p.height)
                    .ok_or_else(|| UiError::new("error.image.decode_failed"))?;
                let source = PhotoSource {
                    image,
                    crop_x: p.crop_x,
                    crop_y: p.crop_y,
                    crop_width: p.crop_width,
                    crop_height: p.crop_height,
                    rotation_deg: p.rotation_deg,
                };
                render_sheet_with_photo(&sheet, printable, &params, &source)
            }
            None => render_sheet(&sheet, printable, &params),
        };

        let job = PrintJob {
            printer: req.printer.clone(),
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
        let _ = (req, pixels);
        Err(UiError::new("error.platform.unsupported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a payload the way the wire does, since it has no constructor.
    fn payload(width: u32, height: u32, mask: Option<(u32, u32)>) -> PhotoPayload {
        let background = match mask {
            Some((mw, mh)) => serde_json::json!({
                "mask_width": mw,
                "mask_height": mh,
                "colour": [255, 255, 255],
            }),
            None => serde_json::Value::Null,
        };
        serde_json::from_value(serde_json::json!({
            "width": width,
            "height": height,
            "crop_x": 0.0,
            "crop_y": 0.0,
            "crop_width": width as f64,
            "crop_height": height as f64,
            "background": background,
        }))
        .expect("payload should deserialise")
    }

    #[test]
    fn a_photo_without_a_mask_takes_the_whole_body() {
        let p = payload(4, 3, None);
        let body = vec![0u8; 4 * 3 * 4];
        let split = split_print_body(&body, &p).expect("exact length should be accepted");
        assert_eq!(split.rgba.len(), 4 * 3 * 4);
        assert!(split.mask.is_empty(), "no mask was declared");
    }

    #[test]
    fn a_mask_is_split_off_after_the_photo() {
        // The two buffers are concatenated with no framing, so the boundary is
        // derived purely from the declared dimensions. Distinct fill bytes
        // prove the split lands in the right place rather than merely summing.
        let p = payload(4, 3, Some((2, 2)));
        let mut body = vec![0xAAu8; 4 * 3 * 4];
        body.extend_from_slice(&[0xBBu8; 2 * 2]);

        let split = split_print_body(&body, &p).expect("exact length should be accepted");
        assert_eq!(split.rgba.len(), 4 * 3 * 4);
        assert_eq!(split.mask.len(), 2 * 2);
        assert!(split.rgba.iter().all(|&b| b == 0xAA), "photo bytes leaked into the mask");
        assert!(split.mask.iter().all(|&b| b == 0xBB), "mask bytes came from the photo");
    }

    #[test]
    fn a_short_body_is_rejected_rather_than_indexed_past_the_end() {
        // The failure this guards: without the length check the renderer would
        // index out of bounds deep inside the resampler.
        let p = payload(4, 3, Some((2, 2)));
        let body = vec![0u8; 4 * 3 * 4]; // mask missing entirely
        let Err(err) = split_print_body(&body, &p) else { panic!("a short body must be refused") };
        assert_eq!(err.key, "error.image.size_mismatch");
        assert_eq!(err.params["expected"], 4 * 3 * 4 + 2 * 2);
        assert_eq!(err.params["got"], 4 * 3 * 4);
    }

    #[test]
    fn a_long_body_is_refused_too() {
        // Extra bytes mean the sender and this side disagree about the layout,
        // so trusting the prefix would print from a buffer built differently.
        let p = payload(4, 3, None);
        let body = vec![0u8; 4 * 3 * 4 + 1];
        let Err(err) = split_print_body(&body, &p) else { panic!("a long body must be refused") };
        assert_eq!(err.key, "error.image.size_mismatch");
    }

    #[test]
    fn absurd_dimensions_are_refused_rather_than_trusted() {
        // Dimensions come off the wire, so the product can overflow. Unchecked
        // it panics in debug and wraps in release, and a wrapped length could
        // match the body and hand the renderer a buffer it would read past.
        let p = payload(u32::MAX, u32::MAX, None);
        let body = vec![0u8; 16];
        let Err(err) = split_print_body(&body, &p) else {
            panic!("absurd dimensions must be refused")
        };
        assert_eq!(err.key, "error.image.size_mismatch");
    }

    #[test]
    fn an_overflowing_mask_is_refused_too() {
        // The mask multiplication is a second, separate overflow site.
        let p = payload(2, 2, Some((u32::MAX, u32::MAX)));
        let body = vec![0u8; 2 * 2 * 4];
        let Err(err) = split_print_body(&body, &p) else {
            panic!("an absurd mask must be refused")
        };
        assert_eq!(err.key, "error.image.size_mismatch");
    }
}
