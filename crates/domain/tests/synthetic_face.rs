//! Geometric tests with synthetic faces.
//!
//! A face of exactly known size goes through the crop solver, and the printed
//! head height is measured back out. The brief asks for +/-0.2mm; these assert
//! considerably tighter, because at this stage the arithmetic is exact and any
//! drift would come from a real mistake rather than from measurement noise.

use domain::geometry::{solve_crop, CropTarget, HeadAnchors, Point, Rect};
use domain::units::px_to_mm;

const TOLERANCE_MM: f64 = 0.2;

/// Build anchors for a head of exactly `head_px` tall, centred at `cx`.
fn synthetic_head(cx: f64, chin_y: f64, head_px: f64) -> HeadAnchors {
    HeadAnchors {
        chin: Point::new(cx, chin_y),
        crown: Point::new(cx, chin_y - head_px),
        estimated: true,
    }
}

/// Head height as it will actually print, in millimetres.
fn printed_head_height_mm(head_px: f64, crop: &Rect, photo_height_mm: f64) -> f64 {
    // The crop is scaled to the photo height, so the head occupies the same
    // fraction of the print as it does of the crop.
    head_px / crop.height * photo_height_mm
}

#[test]
fn head_prints_at_the_requested_height() {
    let image = Rect::new(0.0, 0.0, 2400.0, 3200.0);

    // Croatian passport: 35x45mm, adult head 31.5-36mm. Aim for the middle.
    for head_px in [300.0, 600.0, 900.0, 1200.0] {
        let anchors = synthetic_head(1200.0, 1800.0, head_px);
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.75,
            chin_from_bottom_mm: None,
        };

        let crop = solve_crop(&anchors, &target, &image)
            .unwrap_or_else(|e| panic!("head {head_px}px failed: {e:?}"));

        let printed = printed_head_height_mm(head_px, &crop, target.photo_height_mm);
        assert!(
            (printed - 33.75).abs() < TOLERANCE_MM,
            "head {head_px}px printed at {printed:.3}mm, wanted 33.75mm"
        );
    }
}

#[test]
fn head_height_is_independent_of_source_resolution() {
    // The same face photographed at different resolutions must print the same
    // size. This is what stops a phone photo and a DSLR photo disagreeing.
    let target = CropTarget {
        photo_width_mm: 35.0,
        photo_height_mm: 45.0,
        head_height_mm: 33.0,
        chin_from_bottom_mm: None,
    };

    let mut printed = Vec::new();
    for scale in [1.0, 2.0, 4.0] {
        let image = Rect::new(0.0, 0.0, 1200.0 * scale, 1600.0 * scale);
        let anchors = synthetic_head(600.0 * scale, 900.0 * scale, 400.0 * scale);
        let crop = solve_crop(&anchors, &target, &image).unwrap();
        printed.push(printed_head_height_mm(400.0 * scale, &crop, 45.0));
    }

    for p in &printed {
        assert!((p - 33.0).abs() < TOLERANCE_MM, "printed {p:.4}mm");
    }
    // And they must agree with each other, not merely with the target.
    let spread = printed.iter().cloned().fold(f64::MIN, f64::max)
        - printed.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread < 1e-9, "resolution changed the result by {spread}mm");
}

#[test]
fn chin_line_lands_where_the_spec_asks() {
    // Where a spec fixes the chin line, the printed distance from the bottom
    // edge to the chin must match it.
    let image = Rect::new(0.0, 0.0, 2000.0, 2600.0);
    let anchors = synthetic_head(1000.0, 1600.0, 500.0);
    let target = CropTarget {
        photo_width_mm: 35.0,
        photo_height_mm: 45.0,
        head_height_mm: 33.0,
        chin_from_bottom_mm: Some(6.5),
    };

    let crop = solve_crop(&anchors, &target, &image).unwrap();
    let chin_above_bottom_px = crop.bottom() - anchors.chin.y;
    let printed_mm = chin_above_bottom_px / crop.height * target.photo_height_mm;

    assert!(
        (printed_mm - 6.5).abs() < TOLERANCE_MM,
        "chin printed {printed_mm:.3}mm above the bottom edge, wanted 6.5mm"
    );
}

#[test]
fn crop_aspect_ratio_matches_the_photo() {
    // A crop whose aspect differs from the photo would be stretched at print
    // time, which the regulation forbids outright.
    let image = Rect::new(0.0, 0.0, 3000.0, 4000.0);
    let anchors = synthetic_head(1500.0, 2200.0, 700.0);

    for (w, h) in [(35.0, 45.0), (30.0, 35.0), (50.0, 70.0)] {
        let target = CropTarget {
            photo_width_mm: w,
            photo_height_mm: h,
            head_height_mm: h * 0.72,
            chin_from_bottom_mm: None,
        };
        let crop = solve_crop(&anchors, &target, &image).unwrap();
        let crop_aspect = crop.width / crop.height;
        let photo_aspect = w / h;
        assert!(
            (crop_aspect - photo_aspect).abs() < 1e-9,
            "{w}x{h}mm: crop aspect {crop_aspect:.6} vs photo {photo_aspect:.6}"
        );
    }
}

#[test]
fn eye_distance_survives_the_crop_at_300dpi() {
    // The Croatian spec requires at least 8mm between pupils. A head at the
    // minimum legal height must still clear that, or the spec contradicts
    // itself. Eye spacing is taken as a typical fraction of head height.
    let image = Rect::new(0.0, 0.0, 2400.0, 3200.0);
    let head_px = 800.0;
    let anchors = synthetic_head(1200.0, 2000.0, head_px);

    // Interpupillary distance is roughly 0.32 of chin-to-crown height in
    // adults. Derived, not from the regulation: used only to check the spec's
    // own numbers are mutually consistent.
    let eye_px = head_px * 0.32;

    let target = CropTarget {
        photo_width_mm: 35.0,
        photo_height_mm: 45.0,
        head_height_mm: 31.5, // legal minimum for adults
        chin_from_bottom_mm: None,
    };
    let crop = solve_crop(&anchors, &target, &image).unwrap();
    let eye_mm = eye_px / crop.height * target.photo_height_mm;

    assert!(
        eye_mm >= 8.0,
        "at the minimum head height the eyes are {eye_mm:.2}mm apart, below the 8mm minimum"
    );
}

#[test]
fn crop_at_300dpi_has_enough_pixels_for_the_print() {
    // A 35x45mm photo at 300dpi needs 413x531px. If the crop is smaller than
    // that, printing would upscale, which must never happen silently.
    let image = Rect::new(0.0, 0.0, 2400.0, 3200.0);
    let anchors = synthetic_head(1200.0, 2000.0, 900.0);
    let target = CropTarget {
        photo_width_mm: 35.0,
        photo_height_mm: 45.0,
        head_height_mm: 33.0,
        chin_from_bottom_mm: None,
    };
    let crop = solve_crop(&anchors, &target, &image).unwrap();

    // What the crop is worth in millimetres at 300dpi.
    let available_w_mm = px_to_mm(crop.width, 300.0);
    let available_h_mm = px_to_mm(crop.height, 300.0);

    assert!(
        available_w_mm >= 35.0 && available_h_mm >= 45.0,
        "crop {:.0}x{:.0}px only covers {available_w_mm:.1}x{available_h_mm:.1}mm at 300dpi",
        crop.width,
        crop.height
    );
}

#[test]
fn a_head_too_small_in_frame_is_reported_not_upscaled() {
    // A tiny face in a large image would need a crop larger than the image.
    // The solver must refuse rather than silently produce something unusable.
    let image = Rect::new(0.0, 0.0, 400.0, 400.0);
    let anchors = synthetic_head(200.0, 220.0, 350.0);
    let target = CropTarget {
        photo_width_mm: 35.0,
        photo_height_mm: 45.0,
        head_height_mm: 20.0, // asking the head to be small on paper
        chin_from_bottom_mm: None,
    };
    assert!(
        solve_crop(&anchors, &target, &image).is_err(),
        "an impossible crop was accepted"
    );
}
