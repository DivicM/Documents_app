//! Geometry checks for the calibration sheet, using the real numbers this
//! machine's Brother HL-L2402D reports: 600x600 dpi, 4.23mm hardware margins.
//!
//! These pin down what actually lands on paper, which is otherwise only
//! observable by printing and measuring with a ruler.

use domain::layout::{Orientation, Placement, Sheet, SizeMm};
use domain::render::{placement_to_pixels, render_sheet, PrintableOrigin, RenderParams};

const DPI: f64 = 600.0;
const MARGIN_MM: f64 = 4.23;

fn square_50mm(x_mm: f64, y_mm: f64) -> Placement {
    Placement {
        x_mm,
        y_mm,
        size: SizeMm::new(50.0, 50.0),
        orientation: Orientation::Portrait,
    }
}

#[test]
fn fifty_mm_square_is_1181_px_at_600dpi() {
    // 50mm / 25.4 * 600 = 1181.1. This is the number a ruler measures against.
    let r = placement_to_pixels(&square_50mm(0.0, 0.0), DPI, DPI);
    assert_eq!(r.width, 1181);
    assert_eq!(r.height, 1181);

    // Round-trip: those pixels back to mm must land within a tenth of a mm.
    let back_mm = r.width as f64 * 25.4 / DPI;
    assert!((back_mm - 50.0).abs() < 0.1, "square would print at {back_mm}mm");
}

#[test]
fn hardware_margin_is_subtracted_exactly_once() {
    // A square placed 20mm from the paper edge, on a printer whose printable
    // area starts 4.23mm in, must sit 15.77mm into the printable area.
    let placed = square_50mm(20.0, 20.0);
    let shifted = Placement {
        x_mm: placed.x_mm - MARGIN_MM,
        y_mm: placed.y_mm - MARGIN_MM,
        ..placed
    };
    let r = placement_to_pixels(&shifted, DPI, DPI);

    let expected_mm = 20.0 - MARGIN_MM;
    let actual_mm = r.x as f64 * 25.4 / DPI;
    assert!(
        (actual_mm - expected_mm).abs() < 0.05,
        "expected {expected_mm}mm from printable edge, got {actual_mm}mm"
    );
}

#[test]
fn square_lands_fully_inside_the_printable_raster() {
    // The whole point: nothing may be clipped by the unprintable border.
    let paper = SizeMm::new(210.0, 297.0);
    let printable = SizeMm::new(
        paper.width - 2.0 * MARGIN_MM,
        paper.height - 2.0 * MARGIN_MM,
    );
    let sheet = Sheet {
        placements: vec![square_50mm(20.0, 20.0)],
        orientation: Orientation::Portrait,
        capacity_per_sheet: 1,
        sheets_needed: 1,
    };

    let mut params = RenderParams::new(DPI, DPI);
    params.origin = PrintableOrigin { left_mm: MARGIN_MM, top_mm: MARGIN_MM };
    let raster = render_sheet(&sheet, printable, &params);

    let shifted = Placement {
        x_mm: 20.0 - MARGIN_MM,
        y_mm: 20.0 - MARGIN_MM,
        ..square_50mm(20.0, 20.0)
    };
    let r = placement_to_pixels(&shifted, DPI, DPI);

    assert!(
        r.x + r.width <= raster.width_px,
        "square runs off the right edge: {} > {}",
        r.x + r.width,
        raster.width_px
    );
    assert!(
        r.y + r.height <= raster.height_px,
        "square runs off the bottom edge: {} > {}",
        r.y + r.height,
        raster.height_px
    );
}

#[test]
fn corners_of_the_square_are_black_and_the_surround_is_white() {
    // Verifies the raster really contains the square where the geometry says,
    // not merely that the arithmetic is self-consistent.
    let sheet = Sheet {
        placements: vec![square_50mm(20.0, 20.0)],
        orientation: Orientation::Portrait,
        capacity_per_sheet: 1,
        sheets_needed: 1,
    };
    let raster = render_sheet(&sheet, SizeMm::new(100.0, 100.0), &RenderParams::new(DPI, DPI));

    let px = |mm: f64| (mm / 25.4 * DPI).round() as u32;
    let at = |x: u32, y: u32| -> [u8; 4] {
        let o = (y as usize * raster.width_px as usize + x as usize) * 4;
        [raster.pixels[o], raster.pixels[o + 1], raster.pixels[o + 2], raster.pixels[o + 3]]
    };

    // Just inside each corner of the 20..70mm square.
    assert_eq!(at(px(20.5), px(20.5)), [0, 0, 0, 255], "top-left not filled");
    assert_eq!(at(px(69.5), px(69.5)), [0, 0, 0, 255], "bottom-right not filled");
    // Just outside.
    assert_eq!(at(px(19.5), px(19.5)), [255, 255, 255, 255], "leaked above-left");
    assert_eq!(at(px(70.5), px(70.5)), [255, 255, 255, 255], "leaked below-right");
}

#[test]
fn a4_at_600dpi_matches_the_drivers_own_arithmetic() {
    // 210x297mm minus 4.23mm on each side, at 600dpi.
    let printable = SizeMm::new(210.0 - 2.0 * MARGIN_MM, 297.0 - 2.0 * MARGIN_MM);
    let sheet = Sheet {
        placements: vec![],
        orientation: Orientation::Portrait,
        capacity_per_sheet: 0,
        sheets_needed: 0,
    };
    let raster = render_sheet(&sheet, printable, &RenderParams::new(DPI, DPI));

    // 201.54mm / 25.4 * 600 = 4760.7874 -> 4761
    assert_eq!(raster.width_px, 4761);
    // 288.54mm / 25.4 * 600 = 6815.9055 -> 6816
    assert_eq!(raster.height_px, 6816);
}
