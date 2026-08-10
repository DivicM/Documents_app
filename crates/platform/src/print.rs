//! Print backend abstraction.
//!
//! The contract is deliberately narrow: query what the device can do, then hand
//! it a raster already at the device's own resolution. Nothing here asks a
//! driver to scale, because driver scaling is not reliably controllable (see
//! the note on `PrintJob::pixels`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterInfo {
    pub name: String,
    pub driver: String,
    pub is_default: bool,
}

/// The sheet the driver believes is loaded, and how much of it can be printed.
///
/// Read from the device rather than assumed: a photo printer is configured with
/// one paper size, and printing a layout computed for a different one puts the
/// photos off the edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DevicePaper {
    /// Whole sheet, including any unprintable border.
    pub physical_width_mm: f64,
    pub physical_height_mm: f64,
    /// The area the printer can actually mark.
    pub printable_width_mm: f64,
    pub printable_height_mm: f64,
}

/// Non-printable border the hardware imposes, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub left_mm: f64,
    pub top_mm: f64,
    pub right_mm: f64,
    pub bottom_mm: f64,
}

impl Margins {
    pub const ZERO: Self =
        Self { left_mm: 0.0, top_mm: 0.0, right_mm: 0.0, bottom_mm: 0.0 };
}

/// Physical device resolution, dots per inch, horizontal and vertical.
///
/// Kept as a pair because they are not always equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDpi {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaperSize {
    pub width_mm: f64,
    pub height_mm: f64,
}

/// A sheet ready to print.
///
/// `pixels` must already be at the device resolution reported by
/// [`PrintBackend::device_dpi`]. The backend blits it 1:1 rather than asking the
/// driver to fit it to the page: "fit to page" usually lives in the
/// driver-private part of DEVMODE, `dmScale` is widely ignored, and neither can
/// be set portably. Sizing the raster ourselves is the only way to know what
/// actually lands on paper.
#[derive(Debug, Clone)]
pub struct PrintJob {
    pub printer: String,
    /// BGRA, 8 bits per channel, top-down, `width_px * 4` bytes per row.
    pub pixels: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    pub document_name: String,
}

impl PrintJob {
    /// Bytes a valid buffer must contain.
    pub fn expected_len(&self) -> usize {
        self.width_px as usize * self.height_px as usize * 4
    }

    pub fn validate(&self) -> Result<()> {
        if self.width_px == 0 || self.height_px == 0 {
            return Err(PrintError::EmptyRaster);
        }
        if self.pixels.len() != self.expected_len() {
            return Err(PrintError::RasterSizeMismatch {
                expected: self.expected_len(),
                got: self.pixels.len(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    PrinterNotFound(String),
    EmptyRaster,
    RasterSizeMismatch { expected: usize, got: usize },
    /// A Win32/CUPS call failed; carries the OS message.
    Backend(String),
    Unsupported(&'static str),
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrinterNotFound(n) => write!(f, "printer not found: {n}"),
            Self::EmptyRaster => write!(f, "raster has zero width or height"),
            Self::RasterSizeMismatch { expected, got } => {
                write!(f, "raster size mismatch: expected {expected} bytes, got {got}")
            }
            Self::Backend(m) => write!(f, "print backend error: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

impl std::error::Error for PrintError {}

pub type Result<T> = std::result::Result<T, PrintError>;

pub trait PrintBackend {
    fn list_printers(&self) -> Result<Vec<PrinterInfo>>;

    /// Physical resolution the device actually prints at.
    fn device_dpi(&self, printer: &str) -> Result<DeviceDpi>;

    /// Non-printable border the hardware imposes for the given paper.
    fn hardware_margins_mm(&self, printer: &str, paper: PaperSize) -> Result<Margins>;

    /// The paper the driver is currently configured for.
    fn device_paper(&self, printer: &str) -> Result<DevicePaper>;

    /// Send a raster that is already at device resolution.
    fn print_raster(&self, job: &PrintJob) -> Result<JobId>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(w: u32, h: u32, len: usize) -> PrintJob {
        PrintJob {
            printer: "test".into(),
            pixels: vec![0u8; len],
            width_px: w,
            height_px: h,
            document_name: "test".into(),
        }
    }

    #[test]
    fn correctly_sized_raster_validates() {
        assert_eq!(job(10, 10, 400).validate(), Ok(()));
    }

    #[test]
    fn zero_dimension_rejected() {
        assert_eq!(job(0, 10, 0).validate(), Err(PrintError::EmptyRaster));
    }

    #[test]
    fn short_buffer_rejected() {
        // A truncated buffer would make StretchDIBits read past the end.
        assert_eq!(
            job(10, 10, 399).validate(),
            Err(PrintError::RasterSizeMismatch { expected: 400, got: 399 })
        );
    }
}
