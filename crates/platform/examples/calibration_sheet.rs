//! Renders the 50x50mm calibration square and, with --print, sends it to a
//! printer. Measure the printed square with a ruler: the difference between
//! 50mm and what you measure is the scale error the calibration corrects.
//!
//!   cargo run -p platform --example calibration_sheet
//!   cargo run -p platform --example calibration_sheet -- --print "Printer Name"
//!
//! Without --print nothing is sent to any printer.

#[cfg(windows)]
fn main() {
    use domain::layout::{Orientation, Placement, Sheet, SizeMm};
    use domain::render::{render_sheet, PrintableOrigin, RenderParams};
    use platform::print::{PaperSize, PrintBackend, PrintJob};
    use platform::WindowsPrintBackend;

    let args: Vec<String> = std::env::args().collect();
    let print_to = args
        .iter()
        .position(|a| a == "--print")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let backend = WindowsPrintBackend::new();

    let printer = match print_to.clone() {
        Some(p) => p,
        None => match backend.list_printers() {
            Ok(ps) => match ps.iter().find(|p| p.is_default).or_else(|| ps.first()) {
                Some(p) => p.name.clone(),
                None => {
                    eprintln!("no printers installed");
                    return;
                }
            },
            Err(e) => {
                eprintln!("could not list printers: {e}");
                return;
            }
        },
    };

    let dpi = match backend.device_dpi(&printer) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not read dpi for {printer}: {e}");
            return;
        }
    };

    // A4, the paper most likely to be loaded in a plain printer.
    let paper = SizeMm::new(210.0, 297.0);
    let margins = backend
        .hardware_margins_mm(&printer, PaperSize { width_mm: paper.width, height_mm: paper.height })
        .unwrap_or(platform::print::Margins::ZERO);

    // One 50x50mm square, 20mm in from the paper corner so it clears the
    // unprintable border on any printer.
    let square = Placement {
        x_mm: 20.0,
        y_mm: 20.0,
        size: SizeMm::new(50.0, 50.0),
        orientation: Orientation::Portrait,
    };
    let sheet = Sheet {
        placements: vec![square],
        orientation: Orientation::Portrait,
        capacity_per_sheet: 1,
        sheets_needed: 1,
    };

    // Render only the printable area, and shift the layout into it.
    let printable = SizeMm::new(
        paper.width - margins.left_mm - margins.right_mm,
        paper.height - margins.top_mm - margins.bottom_mm,
    );
    let origin = PrintableOrigin { left_mm: margins.left_mm, top_mm: margins.top_mm };

    let mut params = RenderParams::new(dpi.x as f64, dpi.y as f64);
    params.origin = origin;
    let raster = render_sheet(&sheet, printable, &params);

    println!("printer:          {printer}");
    println!("device dpi:       {}x{}", dpi.x, dpi.y);
    println!(
        "hardware margins: L{:.2} T{:.2} R{:.2} B{:.2} mm",
        margins.left_mm, margins.top_mm, margins.right_mm, margins.bottom_mm
    );
    println!("printable area:   {:.2}x{:.2} mm", printable.width, printable.height);
    println!("raster:           {}x{} px", raster.width_px, raster.height_px);
    println!("square:           50.00x50.00 mm at 20mm from the paper corner");

    let Some(target) = print_to else {
        println!("\nDry run. Pass --print \"{printer}\" to actually print.");
        return;
    };

    let job = PrintJob {
        printer: target.clone(),
        pixels: raster.pixels,
        width_px: raster.width_px,
        height_px: raster.height_px,
        document_name: "Kalibracija 50x50mm".into(),
    };

    match backend.print_raster(&job) {
        Ok(id) => println!("\nsent to {target}, job id {}", id.0),
        Err(e) => eprintln!("\nprint failed: {e}"),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example is Windows-only");
}
