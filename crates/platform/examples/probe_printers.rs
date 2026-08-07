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
            Ok(dpi) => {
                println!("  device dpi: {}x{}", dpi.x, dpi.y);
                let px_w = (100.0 / 25.4 * dpi.x as f64).round();
                let px_h = (150.0 / 25.4 * dpi.y as f64).round();
                println!("  10x15cm sheet would be {px_w}x{px_h} px at this dpi");
            }
            Err(e) => println!("  device dpi: unavailable ({e})"),
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
