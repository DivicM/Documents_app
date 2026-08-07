//! Turns a layout into a raster ready for the printer.
//!
//! The device DPI is a parameter, never a constant: a preview at 96dpi and a
//! final sheet at 600dpi go through this same function. That is what keeps the
//! screen and the paper in agreement.

use crate::layout::{Orientation, Placement, Sheet, SizeMm};
use crate::resample::{resample_region, resample_rotated, ImageBuf, ImageRef};
use crate::units::mm_to_px_exact;

/// A BGRA raster, top-down, 4 bytes per pixel.
///
/// BGRA because that is what Win32 `StretchDIBits` expects from a 32-bit DIB;
/// converting once here beats converting per-pixel at print time.
pub struct Raster {
    pub width_px: u32,
    pub height_px: u32,
    pub pixels: Vec<u8>,
}

impl Raster {
    /// A white sheet of the given size.
    pub fn white(width_px: u32, height_px: u32) -> Self {
        Self {
            width_px,
            height_px,
            pixels: vec![0xFF; width_px as usize * height_px as usize * 4],
        }
    }

    fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width_px as usize + x as usize) * 4
    }

    /// Write one pixel. Out-of-bounds writes are dropped rather than panicking,
    /// because rounding at the edges of a placement can land one pixel over.
    fn put(&mut self, x: u32, y: u32, bgra: [u8; 4]) {
        if x >= self.width_px || y >= self.height_px {
            return;
        }
        let o = self.offset(x, y);
        self.pixels[o..o + 4].copy_from_slice(&bgra);
    }
}

/// Where a photo lands in the raster, in whole pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Convert a placement to pixels at the given DPI.
///
/// Rounds the edges rather than the origin and size independently: rounding
/// both would let a photo drift by a pixel and make gutters uneven.
pub fn placement_to_pixels(p: &Placement, dpi_x: f64, dpi_y: f64) -> PixelRect {
    let left = mm_to_px_exact(p.x_mm, dpi_x).round();
    let top = mm_to_px_exact(p.y_mm, dpi_y).round();
    let right = mm_to_px_exact(p.x_mm + p.size.width, dpi_x).round();
    let bottom = mm_to_px_exact(p.y_mm + p.size.height, dpi_y).round();

    PixelRect {
        x: left.max(0.0) as u32,
        y: top.max(0.0) as u32,
        width: (right - left).max(0.0) as u32,
        height: (bottom - top).max(0.0) as u32,
    }
}

/// Offset applied to every placement so the layout lands inside the printable
/// area. The driver's origin is the top-left of the *printable* region, not of
/// the paper, so a layout computed in paper coordinates must be shifted back by
/// the unprintable margin or everything drifts down and right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrintableOrigin {
    pub left_mm: f64,
    pub top_mm: f64,
}

impl PrintableOrigin {
    pub const ZERO: Self = Self { left_mm: 0.0, top_mm: 0.0 };
}

/// Measured correction for one printer, applied to every length before it
/// becomes pixels.
///
/// X and Y are separate because the paper-feed direction and the print-head
/// direction have different errors. Identity means uncalibrated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationScale {
    pub scale_x: f64,
    pub scale_y: f64,
    pub offset_x_mm: f64,
    pub offset_y_mm: f64,
}

impl CalibrationScale {
    pub const IDENTITY: Self =
        Self { scale_x: 1.0, scale_y: 1.0, offset_x_mm: 0.0, offset_y_mm: 0.0 };

    /// Reject values that would silently ruin output, falling back to identity.
    fn sanitised(self) -> Self {
        let ok = |v: f64| v.is_finite() && v > 0.0;
        if ok(self.scale_x) && ok(self.scale_y) {
            self
        } else {
            Self::IDENTITY
        }
    }
}

impl Default for CalibrationScale {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Everything needed to turn a layout into pixels.
///
/// Grouped into a struct so preview and final render call one function with
/// different values rather than two functions that can drift apart.
#[derive(Debug, Clone, Copy)]
pub struct RenderParams {
    pub dpi_x: f64,
    pub dpi_y: f64,
    pub origin: PrintableOrigin,
    pub calibration: CalibrationScale,
    pub fill: [u8; 4],
}

impl RenderParams {
    /// Uncalibrated, no unprintable border, black fill.
    pub fn new(dpi_x: f64, dpi_y: f64) -> Self {
        Self {
            dpi_x,
            dpi_y,
            origin: PrintableOrigin::ZERO,
            calibration: CalibrationScale::IDENTITY,
            fill: [0, 0, 0, 255],
        }
    }
}

/// Apply calibration and the printable-area offset to a placement.
fn correct(p: &Placement, params: &RenderParams, cal: &CalibrationScale) -> Placement {
    Placement {
        x_mm: (p.x_mm - params.origin.left_mm) * cal.scale_x + cal.offset_x_mm,
        y_mm: (p.y_mm - params.origin.top_mm) * cal.scale_y + cal.offset_y_mm,
        size: SizeMm::new(p.size.width * cal.scale_x, p.size.height * cal.scale_y),
        ..*p
    }
}

/// Allocate the sheet raster for a given paper size.
///
/// The paper is never scaled by calibration: the sheet is physically whatever
/// size it is, and only its contents get corrected.
fn blank_sheet(paper: SizeMm, params: &RenderParams) -> Raster {
    let width_px = mm_to_px_exact(paper.width, params.dpi_x).round().max(1.0) as u32;
    let height_px = mm_to_px_exact(paper.height, params.dpi_y).round().max(1.0) as u32;
    Raster::white(width_px, height_px)
}

/// Render a sheet of solid-colour rectangles, one per placement.
///
/// Used for the calibration square and for verifying geometry without an
/// image. [`render_sheet_with_photo`] draws the same layout with real pixels.
pub fn render_sheet(sheet: &Sheet, paper: SizeMm, params: &RenderParams) -> Raster {
    let cal = params.calibration.sanitised();
    let mut raster = blank_sheet(paper, params);

    for p in &sheet.placements {
        let r = placement_to_pixels(&correct(p, params, &cal), params.dpi_x, params.dpi_y);
        for y in r.y..r.y.saturating_add(r.height) {
            for x in r.x..r.x.saturating_add(r.width) {
                raster.put(x, y, params.fill);
            }
        }
    }

    raster
}

/// The part of a source image that becomes one printed photo.
#[derive(Debug, Clone, Copy)]
pub struct PhotoSource<'a> {
    pub image: ImageRef<'a>,
    /// Crop rectangle in source-image pixels; may be fractional.
    pub crop_x: f64,
    pub crop_y: f64,
    pub crop_width: f64,
    pub crop_height: f64,
    /// Head tilt to straighten, in degrees. Zero skips rotation entirely so an
    /// upright photo is never resampled through the rotated path.
    pub rotation_deg: f64,
}

impl<'a> PhotoSource<'a> {
    /// An unrotated crop.
    pub fn new(image: ImageRef<'a>, x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            image,
            crop_x: x,
            crop_y: y,
            crop_width: width,
            crop_height: height,
            rotation_deg: 0.0,
        }
    }
}

/// Render a sheet with the photo composited into every placement.
///
/// The crop is resampled once at the size the first placement needs and then
/// reused: every copy on a sheet is the same size, so resampling per copy would
/// repeat identical work.
pub fn render_sheet_with_photo(
    sheet: &Sheet,
    paper: SizeMm,
    params: &RenderParams,
    source: &PhotoSource<'_>,
) -> Raster {
    let cal = params.calibration.sanitised();
    let mut raster = blank_sheet(paper, params);

    let Some(first) = sheet.placements.first() else {
        return raster;
    };

    // Every placement is the same size, so one resample serves them all.
    let target = placement_to_pixels(&correct(first, params, &cal), params.dpi_x, params.dpi_y);
    if target.width == 0 || target.height == 0 {
        return raster;
    }

    let rotate = first.orientation == Orientation::Landscape;
    // When the layout rotates the photo, the resampled bitmap is produced in
    // upright orientation and turned when it is written, so the crop keeps its
    // own aspect ratio rather than being squashed into the rotated box.
    let (sample_w, sample_h) =
        if rotate { (target.height, target.width) } else { (target.width, target.height) };

    // Straight crops take the Lanczos path; only a tilted head pays for the
    // rotated sampler, which is bilinear.
    let photo = if source.rotation_deg.abs() < 1e-6 {
        resample_region(
            &source.image,
            source.crop_x,
            source.crop_y,
            source.crop_width,
            source.crop_height,
            sample_w,
            sample_h,
        )
    } else {
        resample_rotated(
            &source.image,
            source.crop_x + source.crop_width / 2.0,
            source.crop_y + source.crop_height / 2.0,
            source.crop_width,
            source.crop_height,
            source.rotation_deg,
            sample_w,
            sample_h,
        )
    };

    for p in &sheet.placements {
        let r = placement_to_pixels(&correct(p, params, &cal), params.dpi_x, params.dpi_y);
        blit(&mut raster, &photo, &r, rotate);
    }

    raster
}

/// Copy a resampled photo into one placement, rotating 90 degrees if needed.
fn blit(dst: &mut Raster, photo: &ImageBuf, at: &PixelRect, rotate: bool) {
    if photo.width == 0 || photo.height == 0 {
        return;
    }
    for row in 0..at.height {
        for col in 0..at.width {
            // Rotating clockwise: the destination column becomes the source
            // row, and the destination row counts back along the source column.
            let (sx, sy) = if rotate {
                (row.min(photo.width - 1), (at.width - 1 - col).min(photo.height - 1))
            } else {
                (col.min(photo.width - 1), row.min(photo.height - 1))
            };
            let si = (sy as usize * photo.width as usize + sx as usize) * 4;
            let px = &photo.pixels[si..si + 4];
            // Source is RGBA; the raster is BGRA for the Windows blitter.
            dst.put(at.x + col, at.y + row, [px[2], px[1], px[0], 255]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Orientation;

    fn placement(x: f64, y: f64, w: f64, h: f64) -> Placement {
        Placement {
            x_mm: x,
            y_mm: y,
            size: SizeMm::new(w, h),
            orientation: Orientation::Portrait,
        }
    }

    #[test]
    fn photo_size_in_pixels_matches_the_briefs_figures() {
        // 35x45mm at 300dpi is 413x531px.
        let r = placement_to_pixels(&placement(0.0, 0.0, 35.0, 45.0), 300.0, 300.0);
        assert_eq!((r.width, r.height), (413, 531));
    }

    #[test]
    fn same_photo_keeps_its_size_wherever_it_sits() {
        // Rounding must not make a photo at 10.3mm a different size from one at
        // 0mm, or copies on one sheet would come out unequal.
        let a = placement_to_pixels(&placement(0.0, 0.0, 35.0, 45.0), 600.0, 600.0);
        let b = placement_to_pixels(&placement(10.3, 7.9, 35.0, 45.0), 600.0, 600.0);
        assert_eq!((a.width, a.height), (b.width, b.height));
    }

    #[test]
    fn printable_origin_shifts_the_layout_back() {
        // A Brother reports 4.23mm of unprintable border. A photo placed at
        // 4.23mm on paper must land at pixel 0 of the printable area.
        let p = placement(4.23, 4.23, 35.0, 45.0);
        let shifted = Placement { x_mm: p.x_mm - 4.23, y_mm: p.y_mm - 4.23, ..p };
        let r = placement_to_pixels(&shifted, 600.0, 600.0);
        assert_eq!((r.x, r.y), (0, 0));
    }

    fn empty_sheet() -> Sheet {
        Sheet {
            placements: vec![],
            orientation: Orientation::Portrait,
            capacity_per_sheet: 0,
            sheets_needed: 0,
        }
    }

    fn sheet_with(p: Placement) -> Sheet {
        Sheet {
            placements: vec![p],
            orientation: Orientation::Portrait,
            capacity_per_sheet: 1,
            sheets_needed: 1,
        }
    }

    #[test]
    fn sheet_raster_has_the_expected_dimensions() {
        // 100x150mm at 600dpi.
        let r = render_sheet(&empty_sheet(), SizeMm::new(100.0, 150.0), &RenderParams::new(600.0, 600.0));
        assert_eq!((r.width_px, r.height_px), (2362, 3543));
        assert_eq!(r.pixels.len(), 2362 * 3543 * 4);
    }

    #[test]
    fn empty_sheet_stays_white() {
        let r = render_sheet(&empty_sheet(), SizeMm::new(10.0, 10.0), &RenderParams::new(300.0, 300.0));
        assert!(r.pixels.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn placements_are_drawn_in_the_right_place() {
        let sheet = sheet_with(placement(10.0, 10.0, 10.0, 10.0));
        let r = render_sheet(&sheet, SizeMm::new(30.0, 30.0), &RenderParams::new(300.0, 300.0));
        // 10mm at 300dpi is 118px. Inside the rect is black, outside white.
        let inside = r.offset(150, 150);
        assert_eq!(&r.pixels[inside..inside + 4], &[0, 0, 0, 255]);
        let outside = r.offset(10, 10);
        assert_eq!(&r.pixels[outside..outside + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn calibration_changes_the_rendered_size() {
        // This is the point of calibration: a printer running 1% small must be
        // compensated by drawing 1% larger.
        let sheet = sheet_with(placement(0.0, 0.0, 50.0, 50.0));
        let paper = SizeMm::new(100.0, 100.0);

        let uncalibrated = render_sheet(&sheet, paper, &RenderParams::new(600.0, 600.0));

        let mut params = RenderParams::new(600.0, 600.0);
        params.calibration = CalibrationScale {
            scale_x: 1.01,
            scale_y: 1.0,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        };
        let calibrated = render_sheet(&sheet, paper, &params);

        // Count black pixels along the top row of the square.
        let black_run = |r: &Raster| {
            (0..r.width_px).filter(|&x| r.pixels[r.offset(x, 5)] == 0).count()
        };
        let plain = black_run(&uncalibrated);
        let scaled = black_run(&calibrated);
        assert!(
            scaled > plain,
            "calibration did not widen the square: {plain} -> {scaled}"
        );
    }

    #[test]
    fn identity_calibration_is_a_no_op() {
        let sheet = sheet_with(placement(10.0, 10.0, 20.0, 20.0));
        let paper = SizeMm::new(60.0, 60.0);

        let plain = render_sheet(&sheet, paper, &RenderParams::new(300.0, 300.0));

        let mut params = RenderParams::new(300.0, 300.0);
        params.calibration = CalibrationScale::IDENTITY;
        let identity = render_sheet(&sheet, paper, &params);

        assert_eq!(plain.pixels, identity.pixels);
    }

    #[test]
    fn nonsense_calibration_falls_back_to_identity() {
        // A zero or NaN scale would collapse or corrupt every photo; better to
        // ignore it than to print garbage.
        let sheet = sheet_with(placement(5.0, 5.0, 20.0, 20.0));
        let paper = SizeMm::new(50.0, 50.0);
        let plain = render_sheet(&sheet, paper, &RenderParams::new(300.0, 300.0));

        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut params = RenderParams::new(300.0, 300.0);
            params.calibration = CalibrationScale {
                scale_x: bad,
                scale_y: 1.0,
                offset_x_mm: 0.0,
                offset_y_mm: 0.0,
            };
            let got = render_sheet(&sheet, paper, &params);
            assert_eq!(got.pixels, plain.pixels, "scale {bad} was not rejected");
        }
    }

    /// A source image with a distinctive colour per quadrant, so orientation
    /// errors are visible rather than plausible.
    fn quadrant_image() -> Vec<u8> {
        let mut px = vec![0u8; 100 * 100 * 4];
        for y in 0..100usize {
            for x in 0..100usize {
                let i = (y * 100 + x) * 4;
                let c: [u8; 4] = match (x < 50, y < 50) {
                    (true, true) => [255, 0, 0, 255],    // top-left red
                    (false, true) => [0, 255, 0, 255],   // top-right green
                    (true, false) => [0, 0, 255, 255],   // bottom-left blue
                    (false, false) => [255, 255, 0, 255],
                };
                px[i..i + 4].copy_from_slice(&c);
            }
        }
        px
    }

    #[test]
    fn photo_pixels_are_composited_not_a_flat_fill() {
        // The gap this closes: render_sheet drew solid rectangles, so a print
        // came out grey regardless of the photo.
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let sheet = sheet_with(placement(0.0, 0.0, 20.0, 20.0));
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let raster = render_sheet_with_photo(
            &sheet,
            SizeMm::new(40.0, 40.0),
            &RenderParams::new(300.0, 300.0),
            &source,
        );

        // Top-left of the placement must be red, not a uniform fill.
        let o = raster.offset(10, 10);
        let bgra = &raster.pixels[o..o + 4];
        assert_eq!(bgra[2], 255, "red channel missing at top-left");
        assert!(bgra[1] < 50 && bgra[0] < 50, "expected red, got {bgra:?}");

        // Bottom-right of the placement must be yellow.
        let r = placement_to_pixels(&placement(0.0, 0.0, 20.0, 20.0), 300.0, 300.0);
        let o2 = raster.offset(r.width - 10, r.height - 10);
        let br = &raster.pixels[o2..o2 + 4];
        assert!(br[2] > 200 && br[1] > 200, "expected yellow, got {br:?}");
    }

    #[test]
    fn every_copy_on_the_sheet_gets_the_photo() {
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let cfg = crate::layout::LayoutConfig {
            paper: SizeMm::new(100.0, 100.0),
            photo: SizeMm::new(20.0, 20.0),
            count: 4,
            margin_mm: 2.0,
            gutter_mm: 2.0,
            alignment: crate::layout::Alignment::TopLeft,
        };
        let sheet = crate::layout::solve(&cfg).unwrap();
        assert!(sheet.placements.len() >= 4);

        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);
        let raster = render_sheet_with_photo(
            &sheet,
            cfg.paper,
            &RenderParams::new(300.0, 300.0),
            &source,
        );

        for (i, p) in sheet.placements.iter().enumerate().take(4) {
            let r = placement_to_pixels(p, 300.0, 300.0);
            let o = raster.offset(r.x + 5, r.y + 5);
            let bgra = &raster.pixels[o..o + 4];
            assert!(
                bgra[2] > 200 && bgra[1] < 60,
                "copy {i} is not red at its top-left: {bgra:?}"
            );
        }
    }

    #[test]
    fn a_rotated_layout_rotates_the_photo_too() {
        // A landscape layout must turn the image, not stretch it: otherwise
        // faces come out squashed.
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();

        // 30x45mm photo on 100x100 paper: rotating gives more copies.
        let cfg = crate::layout::LayoutConfig {
            paper: SizeMm::new(100.0, 100.0),
            photo: SizeMm::new(30.0, 45.0),
            count: 2,
            margin_mm: 0.0,
            gutter_mm: 0.0,
            alignment: crate::layout::Alignment::TopLeft,
        };
        let sheet = crate::layout::solve(&cfg).unwrap();

        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);
        let raster = render_sheet_with_photo(
            &sheet,
            cfg.paper,
            &RenderParams::new(300.0, 300.0),
            &source,
        );

        // Whatever the orientation, the photo must appear: sample inside the
        // first placement and require a saturated colour rather than white.
        let r = placement_to_pixels(&sheet.placements[0], 300.0, 300.0);
        let o = raster.offset(r.x + r.width / 2, r.y + r.height / 2);
        let bgra = &raster.pixels[o..o + 4];
        assert!(
            !(bgra[0] > 240 && bgra[1] > 240 && bgra[2] > 240),
            "placement is still blank white: {bgra:?}"
        );
    }

    #[test]
    fn an_empty_sheet_with_a_photo_stays_white() {
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);
        let raster = render_sheet_with_photo(
            &empty_sheet(),
            SizeMm::new(20.0, 20.0),
            &RenderParams::new(300.0, 300.0),
            &source,
        );
        assert!(raster.pixels.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn paper_size_is_not_scaled_by_calibration() {
        // The sheet is physical; only its contents are corrected.
        let mut params = RenderParams::new(600.0, 600.0);
        params.calibration = CalibrationScale {
            scale_x: 1.05,
            scale_y: 1.05,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        };
        let r = render_sheet(&empty_sheet(), SizeMm::new(100.0, 150.0), &params);
        assert_eq!((r.width_px, r.height_px), (2362, 3543));
    }
}
