//! Windows print backend, talking to the spooler directly.
//!
//! Deliberately avoids any "print this file" API: those inherit whatever
//! defaults the user's driver happens to carry, including silent scaling. We
//! open a device context, ask it what resolution it really is, and blit a
//! raster of exactly that size 1:1.

use crate::print::{
    DeviceDpi, DevicePaper, JobId, Margins, PaperSize, PrintBackend, PrintError, PrintJob,
    PrinterInfo, Result,
};

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetDeviceCaps, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, GET_DEVICE_CAPS_INDEX, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT,
    PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, SRCCOPY, VERTRES,
};
use windows::Win32::Graphics::Printing::{
    ClosePrinter, EnumPrintersW, OpenPrinterW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL,
    PRINTER_INFO_2W,
};
use windows::Win32::Storage::Xps::{EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};

/// Owns an HDC and deletes it on drop, so early returns cannot leak it.
struct PrinterDc(HDC);

impl PrinterDc {
    fn open(printer: &str) -> Result<Self> {
        let name = to_wide(printer);
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let hdc = unsafe { CreateDCW(PCWSTR::null(), PCWSTR(name.as_ptr()), PCWSTR::null(), None) };
        if hdc.is_invalid() {
            return Err(PrintError::PrinterNotFound(printer.to_string()));
        }
        Ok(Self(hdc))
    }

    fn caps(&self, index: GET_DEVICE_CAPS_INDEX) -> i32 {
        // SAFETY: self.0 is a valid DC for as long as this struct lives.
        unsafe { GetDeviceCaps(self.0, index) }
    }
}

impl Drop for PrinterDc {
    fn drop(&mut self) {
        // SAFETY: created by CreateDCW and not deleted anywhere else.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: the spooler returns NUL-terminated strings inside the buffer we
    // allocated, which is still alive at this point.
    unsafe {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

pub struct WindowsPrintBackend;

impl WindowsPrintBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsPrintBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PrintBackend for WindowsPrintBackend {
    fn list_printers(&self) -> Result<Vec<PrinterInfo>> {
        let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
        let mut needed = 0u32;
        let mut returned = 0u32;

        // First call sizes the buffer; it is expected to fail.
        // SAFETY: passing None with a null buffer is the documented probe form.
        unsafe {
            let _ = EnumPrintersW(flags, PCWSTR::null(), 2, None, &mut needed, &mut returned);
        }
        if needed == 0 {
            return Ok(Vec::new());
        }

        let mut buffer = vec![0u8; needed as usize];
        // SAFETY: buffer is at least `needed` bytes, as the probe requested.
        unsafe {
            EnumPrintersW(
                flags,
                PCWSTR::null(),
                2,
                Some(&mut buffer),
                &mut needed,
                &mut returned,
            )
            .map_err(|e| PrintError::Backend(e.message()))?;
        }

        let default = default_printer_name();
        let mut out = Vec::with_capacity(returned as usize);
        // SAFETY: the spooler filled `returned` PRINTER_INFO_2W records.
        let records = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr() as *const PRINTER_INFO_2W, returned as usize)
        };
        for r in records {
            let name = from_wide(r.pPrinterName.0);
            out.push(PrinterInfo {
                is_default: default.as_deref() == Some(name.as_str()),
                driver: from_wide(r.pDriverName.0),
                name,
            });
        }
        Ok(out)
    }

    fn device_dpi(&self, printer: &str) -> Result<DeviceDpi> {
        let dc = PrinterDc::open(printer)?;
        let x = dc.caps(LOGPIXELSX);
        let y = dc.caps(LOGPIXELSY);
        if x <= 0 || y <= 0 {
            return Err(PrintError::Backend(format!(
                "driver reported nonsensical resolution {x}x{y}"
            )));
        }
        Ok(DeviceDpi { x: x as u32, y: y as u32 })
    }

    fn device_paper(&self, printer: &str) -> Result<DevicePaper> {
        let dc = PrinterDc::open(printer)?;
        let dpi_x = dc.caps(LOGPIXELSX);
        let dpi_y = dc.caps(LOGPIXELSY);
        if dpi_x <= 0 || dpi_y <= 0 {
            return Err(PrintError::Backend("driver reported no resolution".into()));
        }

        let to_mm = |px: i32, dpi: i32| (px as f64) * 25.4 / (dpi as f64);

        Ok(DevicePaper {
            physical_width_mm: to_mm(dc.caps(PHYSICALWIDTH), dpi_x),
            physical_height_mm: to_mm(dc.caps(PHYSICALHEIGHT), dpi_y),
            printable_width_mm: to_mm(dc.caps(HORZRES), dpi_x),
            printable_height_mm: to_mm(dc.caps(VERTRES), dpi_y),
        })
    }

    fn hardware_margins_mm(&self, printer: &str, _paper: PaperSize) -> Result<Margins> {
        let dc = PrinterDc::open(printer)?;
        let dpi_x = dc.caps(LOGPIXELSX);
        let dpi_y = dc.caps(LOGPIXELSY);
        if dpi_x <= 0 || dpi_y <= 0 {
            return Err(PrintError::Backend("driver reported no resolution".into()));
        }

        // PHYSICALWIDTH is the whole sheet; HORZRES only the printable part.
        // The offset is the top/left unprintable border, and the remainder of
        // the difference is the bottom/right one.
        let phys_w = dc.caps(PHYSICALWIDTH);
        let phys_h = dc.caps(PHYSICALHEIGHT);
        let print_w = dc.caps(HORZRES);
        let print_h = dc.caps(VERTRES);
        let off_x = dc.caps(PHYSICALOFFSETX);
        let off_y = dc.caps(PHYSICALOFFSETY);

        let to_mm = |px: i32, dpi: i32| (px as f64) * 25.4 / (dpi as f64);

        Ok(Margins {
            left_mm: to_mm(off_x, dpi_x),
            top_mm: to_mm(off_y, dpi_y),
            right_mm: to_mm((phys_w - print_w - off_x).max(0), dpi_x),
            bottom_mm: to_mm((phys_h - print_h - off_y).max(0), dpi_y),
        })
    }

    fn print_raster(&self, job: &PrintJob) -> Result<JobId> {
        job.validate()?;
        let dc = PrinterDc::open(&job.printer)?;

        let doc_name = to_wide(&job.document_name);
        let info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: PCWSTR(doc_name.as_ptr()),
            ..Default::default()
        };

        // SAFETY: dc is valid; info and doc_name outlive the call.
        let job_id = unsafe { StartDocW(dc.0, &info) };
        if job_id <= 0 {
            return Err(PrintError::Backend("StartDoc failed".into()));
        }

        // From here on a failure must still close the document, otherwise the
        // spooler is left holding an open job.
        let result = (|| -> Result<()> {
            // SAFETY: dc is valid and a document is open.
            if unsafe { StartPage(dc.0) } <= 0 {
                return Err(PrintError::Backend("StartPage failed".into()));
            }

            let header = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: job.width_px as i32,
                // Negative height selects a top-down DIB, matching our buffer.
                biHeight: -(job.height_px as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let bmi = BITMAPINFO { bmiHeader: header, ..Default::default() };

            // Source and destination rectangles are identical: no scaling. This
            // is the whole point of rendering at device resolution.
            // SAFETY: pixels holds width*height*4 bytes (checked by validate).
            let scanlines = unsafe {
                StretchDIBits(
                    dc.0,
                    0,
                    0,
                    job.width_px as i32,
                    job.height_px as i32,
                    0,
                    0,
                    job.width_px as i32,
                    job.height_px as i32,
                    Some(job.pixels.as_ptr() as *const _),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                )
            };
            if scanlines == 0 {
                return Err(PrintError::Backend("StretchDIBits copied nothing".into()));
            }

            // SAFETY: a page is open on a valid dc.
            if unsafe { EndPage(dc.0) } <= 0 {
                return Err(PrintError::Backend("EndPage failed".into()));
            }
            Ok(())
        })();

        // SAFETY: a document was successfully started on this dc.
        unsafe {
            if result.is_err() {
                // Best effort: the job is already broken, so ignore the result.
                let _ = EndDoc(dc.0);
            } else if EndDoc(dc.0) <= 0 {
                return Err(PrintError::Backend("EndDoc failed".into()));
            }
        }

        result.map(|_| JobId(job_id as u32))
    }
}

/// Name of the default printer, if the spooler reports one.
fn default_printer_name() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Graphics::Printing::GetDefaultPrinterW;

    let mut len = 0u32;
    // SAFETY: probe call with a null buffer; documented to fail and set `len`.
    unsafe {
        let _ = GetDefaultPrinterW(PWSTR::null(), &mut len);
    }
    if len == 0 {
        return None;
    }

    let mut buf = vec![0u16; len as usize];
    // SAFETY: buf holds `len` wide chars, exactly what the probe asked for.
    let ok = unsafe { GetDefaultPrinterW(PWSTR(buf.as_mut_ptr()), &mut len) };
    if !ok.as_bool() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Opens and immediately closes a printer handle, to check the name resolves.
pub fn printer_exists(name: &str) -> bool {
    let wide = to_wide(name);
    let mut handle = Default::default();
    // SAFETY: wide is NUL-terminated and outlives the call.
    let opened =
        unsafe { OpenPrinterW(PCWSTR(wide.as_ptr()), &mut handle, None) };
    if opened.is_ok() {
        // SAFETY: handle was successfully opened above.
        unsafe {
            let _ = ClosePrinter(handle);
        }
        true
    } else {
        false
    }
}
