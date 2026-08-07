//! The single place where millimetres become pixels.
//!
//! Everywhere else works in mm. Confining the conversion here means a DPI
//! mistake has exactly one place to hide, and the preview and the final render
//! can share it by passing a different DPI rather than different code.

pub const MM_PER_INCH: f64 = 25.4;

/// Convert millimetres to pixels at a given DPI, rounding to the nearest pixel.
///
/// Rounding rather than truncating matters: 35mm at 300dpi is 413.38px, and
/// truncating every photo on a sheet accumulates a visible drift.
pub fn mm_to_px(mm: f64, dpi: f64) -> u32 {
    debug_assert!(mm >= 0.0, "negative length: {mm}");
    debug_assert!(dpi > 0.0, "non-positive dpi: {dpi}");
    (mm / MM_PER_INCH * dpi).round().max(0.0) as u32
}

/// Convert millimetres to pixels without rounding.
///
/// Use for positions inside a raster, where rounding each coordinate
/// independently would make gutters uneven.
pub fn mm_to_px_exact(mm: f64, dpi: f64) -> f64 {
    debug_assert!(dpi > 0.0, "non-positive dpi: {dpi}");
    mm / MM_PER_INCH * dpi
}

pub fn px_to_mm(px: f64, dpi: f64) -> f64 {
    debug_assert!(dpi > 0.0, "non-positive dpi: {dpi}");
    px * MM_PER_INCH / dpi
}

/// Largest DPI at which `source_px` still covers `target_mm` without upscaling.
///
/// Returned so the UI can tell the user what their image is actually good for
/// instead of silently interpolating.
pub fn max_lossless_dpi(source_px: u32, target_mm: f64) -> f64 {
    debug_assert!(target_mm > 0.0, "non-positive target: {target_mm}");
    source_px as f64 * MM_PER_INCH / target_mm
}

/// Whether rendering `target_mm` at `dpi` would need more pixels than the
/// source has, i.e. whether it would upscale.
pub fn would_upscale(source_px: u32, target_mm: f64, dpi: f64) -> bool {
    mm_to_px(target_mm, dpi) > source_px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passport_photo_at_300dpi() {
        // The figures quoted in the brief: 35x45mm at 300dpi is 413x531px.
        assert_eq!(mm_to_px(35.0, 300.0), 413);
        assert_eq!(mm_to_px(45.0, 300.0), 531);
    }

    #[test]
    fn ten_by_fifteen_sheet_at_300dpi() {
        // The brief's sheet figure: 100x150mm at 300dpi is 1181x1772px.
        assert_eq!(mm_to_px(100.0, 300.0), 1181);
        assert_eq!(mm_to_px(150.0, 300.0), 1772);
    }

    #[test]
    fn round_trip_is_stable() {
        for mm in [1.0, 35.0, 45.0, 100.0, 150.0] {
            let back = px_to_mm(mm_to_px_exact(mm, 600.0), 600.0);
            assert!((back - mm).abs() < 1e-9, "{mm} -> {back}");
        }
    }

    #[test]
    fn upscale_detection_matches_the_briefs_example() {
        // 280x360px source for a 35x45mm photo at 300dpi needs 413x531.
        assert!(would_upscale(280, 35.0, 300.0));
        assert!(!would_upscale(413, 35.0, 300.0));
        // Exactly enough is not upscaling.
        assert!(!would_upscale(413, 35.0, 300.0));
    }

    #[test]
    fn max_lossless_dpi_is_the_inverse_of_mm_to_px() {
        let dpi = max_lossless_dpi(413, 35.0);
        assert!(mm_to_px(35.0, dpi) <= 413);
        assert!((dpi - 299.7).abs() < 0.5, "unexpected dpi: {dpi}");
    }
}
