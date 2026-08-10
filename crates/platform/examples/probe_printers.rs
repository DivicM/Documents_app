//! Prints what the spooler reports about every installed printer.
//!
//! Run with: cargo run -p platform --example probe_printers
//! Read-only: it opens device contexts and queries them, nothing is printed.

#[cfg(windows)]
fn main() {
    use platform::print::{PaperSize, PrintBackend};
    use platform::WindowsPrintBackend;

    let backend = WindowsPrintBackend::new();
    let printers = match backend.list_printers() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("could not list printers: {e}");
            std::process::exit(1);
        }
    };

    if printers.is_empty() {
        println!("no printers installed");
        return;
    }

    // 10x15cm, the common photo paper size.
    let paper = PaperSize { width_mm: 100.0, height_mm: 150.0 };

    for p in &printers {
        let marker = if p.is_default { " (default)" } else { "" };
        println!("\n{}{}", p.name, marker);
        println!("  driver: {}", p.driver);

        match backend.device_dpi(&p.name) {
            Ok(dpi) => println!("  device dpi: {}x{}", dpi.x, dpi.y),
            Err(e) => println!("  device dpi: unavailable ({e})"),
        }

        // The paper the driver is set to, which is what the layout must match.
        // Assuming 10x15 and being wrong is how photos end up off the sheet.
        match backend.device_paper(&p.name) {
            Ok(dp) => {
                println!(
                    "  CONFIGURED PAPER: {:.1} x {:.1} mm (printable {:.1} x {:.1} mm)",
                    dp.physical_width_mm,
                    dp.physical_height_mm,
                    dp.printable_width_mm,
                    dp.printable_height_mm
                );
            }
            Err(e) => println!("  configured paper: unavailable ({e})"),
        }

        match backend.hardware_margins_mm(&p.name, paper) {
            Ok(m) => println!(
                "  hardware margins mm: left {:.2}, top {:.2}, right {:.2}, bottom {:.2}",
                m.left_mm, m.top_mm, m.right_mm, m.bottom_mm
            ),
            Err(e) => println!("  hardware margins: unavailable ({e})"),
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example is Windows-only");
}
