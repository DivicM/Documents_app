//! Platform layer: printers, files, config. Everything OS-specific lives here
//! so the layers above it stay testable without a device.

pub mod calibration;
pub mod print;

#[cfg(windows)]
pub mod print_win;

#[cfg(windows)]
pub use print_win::WindowsPrintBackend;

pub use calibration::{Calibration, CalibrationError, CalibrationKey, CalibrationStore};
pub use print::{
    DeviceDpi, JobId, Margins, PaperSize, PrintBackend, PrintError, PrintJob, PrinterInfo,
};
