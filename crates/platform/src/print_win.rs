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

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetDeviceCaps, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DEVMODEW, DIB_RGB_COLORS, DM_OUT_BUFFER, DM_PRINTQUALITY, DM_YRESOLUTION,
    GET_DEVICE_CAPS_INDEX, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX,
    PHYSICALOFFSETY, PHYSICALWIDTH, SRCCOPY, VERTRES,
};
use windows::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EnumPrintersW, OpenPrinterW, PRINTER_ENUM_CONNECTIONS,
    PRINTER_ENUM_LOCAL, PRINTER_INFO_2W,
};
use windows::Win32::Storage::Xps::{
    DeviceCapabilitiesW, EndDoc, EndPage, StartDocW, StartPage, DC_ENUMRESOLUTIONS, DOCINFOW,
};

/// Owns an HDC and deletes it on drop, so early returns cannot leak it.
struct PrinterDc(HDC);

impl PrinterDc {
    fn open(printer: &str) -> Result<Self> {
        Self::open_with_dpi(printer, None)
    }

    /// Open a device context, optionally asking the driver for a resolution.
    ///
    /// The driver's own setting is used when `dpi` is `None`. A requested
    /// resolution is passed through DEVMODE, which is the only way to print at
    /// anything but whatever the Windows printer dialog was last left on — and
    /// unlike `dmScale`, the resolution fields are honoured by real drivers.
    /// A driver that ignores them simply prints at its current setting, which
    /// `device_dpi` then reports, so the raster still matches the paper.
    fn open_with_dpi(printer: &str, dpi: Option<DeviceDpi>) -> Result<Self> {
        let name = to_wide(printer);

        let mut devmode = match dpi {
            Some(d) => printer_devmode(printer).map(|mut dm| {
                // dmPrintQuality carries the horizontal resolution and lives in
                // DEVMODE's printer-side union; dmYResolution is a plain field.
                dm.Anonymous1.Anonymous1.dmPrintQuality = clamp_dpi(d.x);
                dm.dmYResolution = clamp_dpi(d.y);
                dm.dmFields |= DM_PRINTQUALITY | DM_YRESOLUTION;
                dm
            }),
            None => None,
        };

        // SAFETY: `name` is a NUL-terminated wide string that outlives the call,
        // and `devmode`, when present, is a DEVMODEW the driver itself sized.
        let hdc = unsafe {
            CreateDCW(
                PCWSTR::null(),
                PCWSTR(name.as_ptr()),
                PCWSTR::null(),
                devmode.as_mut().map(|dm| dm as *const DEVMODEW),
            )
        };
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

/// DEVMODE stores resolution in an `i16`, so a larger value cannot be asked for.
fn clamp_dpi(dpi: u32) -> i16 {
    dpi.min(i16::MAX as u32) as i16
}

/// The driver's current DEVMODE, as the starting point for a change.
///
/// Built from the driver rather than zeroed: DEVMODE carries driver-private
/// data past its public fields, and discarding it would reset settings the user
/// made in the Windows dialog.
fn printer_devmode(printer: &str) -> Option<DEVMODEW> {
    let name = to_wide(printer);

    // Size first: the public struct is only the start of what the driver keeps.
    // SAFETY: `name` outlives the call; a null output asks for the size.
    let needed = unsafe {
        DocumentPropertiesW(None, None, PCWSTR(name.as_ptr()), None, None, 0)
    };
    if needed <= 0 {
        return None;
    }

    let mut buffer = vec![0u8; needed as usize];
    let dm = buffer.as_mut_ptr().cast::<DEVMODEW>();
    // SAFETY: the buffer is exactly the size the driver asked for, and
    // DM_OUT_BUFFER tells it to fill that buffer with the current settings.
    let rc = unsafe {
        DocumentPropertiesW(
            None,
            None,
            PCWSTR(name.as_ptr()),
            Some(dm),
            None,
            DM_OUT_BUFFER.0,
        )
    };
    if rc < 0 {
        return None;
    }
    // SAFETY: the driver filled at least a full DEVMODEW.
    Some(unsafe { *dm })
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

    fn device_dpi_for(&self, printer: &str, requested: Option<DeviceDpi>) -> Result<DeviceDpi> {
        let dc = PrinterDc::open_with_dpi(printer, requested)?;
        let x = dc.caps(LOGPIXELSX);
        let y = dc.caps(LOGPIXELSY);
        if x <= 0 || y <= 0 {
            return Err(PrintError::Backend(format!(
                "driver reported nonsensical resolution {x}x{y}"
            )));
        }
        Ok(DeviceDpi { x: x as u32, y: y as u32 })
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

    fn available_dpi(&self, printer: &str) -> Result<Vec<DeviceDpi>> {
        let name = to_wide(printer);

        // Asked twice: once for the count, once for the values. A driver that
        // enumerates none returns -1, which is not an error — it simply has no
        // list to offer, and the current setting stands.
        // SAFETY: `name` is NUL-terminated and outlives both calls; passing a
        // null output buffer is how the count is requested.
        let count = unsafe {
            DeviceCapabilitiesW(
                PCWSTR(name.as_ptr()),
                PCWSTR::null(),
                DC_ENUMRESOLUTIONS,
                PWSTR::null(),
                None,
            )
        };
        if count <= 0 {
            return Ok(Vec::new());
        }

        // Each entry is a pair of i32: horizontal then vertical dpi.
        let mut pairs = vec![0i32; count as usize * 2];
        // SAFETY: the buffer holds exactly the pair count the call above
        // reported, which is what the driver writes.
        let written = unsafe {
            DeviceCapabilitiesW(
                PCWSTR(name.as_ptr()),
                PCWSTR::null(),
                DC_ENUMRESOLUTIONS,
                PWSTR(pairs.as_mut_ptr().cast()),
                None,
            )
        };
        if written <= 0 {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        for pair in pairs.chunks_exact(2).take(written as usize) {
            let (x, y) = (pair[0], pair[1]);
            if x > 0 && y > 0 {
                out.push(DeviceDpi { x: x as u32, y: y as u32 });
            }
        }
        out.sort_by_key(|d| (d.x, d.y));
        out.dedup();
        Ok(out)
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
        let dc = PrinterDc::open_with_dpi(&job.printer, job.dpi)?;

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
