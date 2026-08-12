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
    /// Turn the photo inside its frame a quarter turn.
    ///
    /// Distinct from the frame's own orientation: the layout decides how the
    /// rectangle sits on the paper, this decides which way up the picture is
    /// inside it. Setting both gives a sideways face in a sideways frame.
    pub turn_photo: bool,
    /// Draw faint guides showing where each photo ends, for cutting by hand.
    ///
    /// Drawn in the paper margin and gutters only, never across a photograph:
    /// a line over the picture would be printed into the finished photo and
    /// could not be trimmed away.
    pub cut_marks: bool,
}

/// The grey the cutting guides are drawn in.
///
/// 198, a 22% grey: clearly visible under normal light, but far enough from
/// black that an inaccurate cut leaves no obvious dark edge on the photograph.
///
/// Settled by printing. 64 (75%) read as a hard line, 226 (11%) was weak on
/// paper, 190 (25%) was right but a shade heavy; this is one step back from
/// it. Two earlier attempts at a pale tone printed as nothing at all, but both
/// were a single pixel wide — 0.085mm at 300dpi — and a photo printer
/// halftones something that thin and that pale into no ink. The line is now
/// [`CUT_MARK_THICKNESS_MM`] wide, which is what lets a light tone survive at
/// all. Adjust this value rather than the thickness if the guide ever needs to
/// change again.
const CUT_MARK_GREY: u8 = 198;

/// Thickness of each guide line, in millimetres.
///
/// Specified in millimetres rather than pixels because that is what reaches
/// the paper: a one-pixel line is 0.085mm at 300dpi and 0.042mm at 600dpi, so
/// it grew fainter the better the printer, and vanished entirely on a
/// dye-sublimation unit. 0.35mm reads clearly at arm's length and is still
/// narrow enough to cut along accurately. Much below 0.3mm the line starts
/// losing dither cells again and prints unevenly, so this is near the floor
/// rather than a free parameter.
const CUT_MARK_THICKNESS_MM: f64 = 0.35;

impl RenderParams {
    /// Uncalibrated, no unprintable border, black fill.
    pub fn new(dpi_x: f64, dpi_y: f64) -> Self {
        Self {
            dpi_x,
            dpi_y,
            origin: PrintableOrigin::ZERO,
            calibration: CalibrationScale::IDENTITY,
            fill: [0, 0, 0, 255],
            turn_photo: false,
            cut_marks: false,
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

    // XOR: the frame's own rotation and a requested turn compose, so turning a
    // photo inside an already-rotated frame puts it back upright.
    let rotate = (first.orientation == Orientation::Landscape) != params.turn_photo;
    // When the layout rotates the photo, the resampled bitmap is produced in
    // upright orientation and turned when it is written, so the crop keeps its
    // own aspect ratio rather than being squashed into the rotated box.
    let (sample_w, sample_h) =
        if rotate { (target.height, target.width) } else { (target.width, target.height) };

    // Straight crops take the Lanczos path; only a tilted head pays for the
    // rotated sampler, which is bilinear.
    let photo = sample_crop(source, sample_w, sample_h);

    for p in &sheet.placements {
        let r = placement_to_pixels(&correct(p, params, &cal), params.dpi_x, params.dpi_y);
        blit(&mut raster, &photo, &r, rotate);
        if params.cut_marks {
            draw_cut_marks(&mut raster, &r, params.dpi_x, params.dpi_y);
        }
    }

    raster
}

/// Render a mixed sheet: several photo sizes, all from the same crop.
///
/// Unlike [`render_sheet_with_photo`], the placements are not all one size, so
/// the single-resample trick does not apply. Each distinct pixel size is
/// resampled once and reused for every placement that needs it, which keeps the
/// common case — a handful of copies per size — down to one resample per size.
pub fn render_mixed_sheet(
    placements: &[Placement],
    paper: SizeMm,
    params: &RenderParams,
    source: &PhotoSource<'_>,
) -> Raster {
    let cal = params.calibration.sanitised();
    let mut raster = blank_sheet(paper, params);

    // Keyed by the sampled bitmap size and whether it is turned, which is
    // exactly what determines the pixels produced.
    let mut cache: Vec<((u32, u32, bool), ImageBuf)> = Vec::new();

    for p in placements {
        let r = placement_to_pixels(&correct(p, params, &cal), params.dpi_x, params.dpi_y);
        if r.width == 0 || r.height == 0 {
            continue;
        }

        let rotate = (p.orientation == Orientation::Landscape) != params.turn_photo;
        let (sample_w, sample_h) = if rotate { (r.height, r.width) } else { (r.width, r.height) };
        let key = (sample_w, sample_h, rotate);

        if !cache.iter().any(|(k, _)| *k == key) {
            cache.push((key, sample_crop(source, sample_w, sample_h)));
        }
        let photo = cache
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, img)| img)
            .expect("just inserted");

        blit(&mut raster, photo, &r, rotate);
        if params.cut_marks {
            draw_cut_marks(&mut raster, &r, params.dpi_x, params.dpi_y);
        }
    }

    raster
}

/// Resample the source crop to an exact pixel size.
///
/// Split out so the single-size and mixed paths cannot drift apart: a change to
/// how a crop is sampled applies to both.
fn sample_crop(source: &PhotoSource<'_>, width: u32, height: u32) -> ImageBuf {
    if source.rotation_deg.abs() < 1e-6 {
        resample_region(
            &source.image,
            source.crop_x,
            source.crop_y,
            source.crop_width,
            source.crop_height,
            width,
            height,
        )
    } else {
        resample_rotated(
            &source.image,
            source.crop_x + source.crop_width / 2.0,
            source.crop_y + source.crop_height / 2.0,
            source.crop_width,
            source.crop_height,
            source.rotation_deg,
            width,
            height,
        )
    }
}

/// Draw a cutting frame immediately outside one photograph.
///
/// A closed rectangle around the picture rather than lines across the sheet:
/// sheet-wide lines carried on through the empty part of the paper and drew a
/// grid of cells where no photograph was, which is noise to cut around rather
/// than a guide.
///
/// The band sits entirely outside the placement, so trimming along its inner
/// edge removes the guide with the waste. Nothing is written over the picture,
/// which means the photographs do not have to be drawn again afterwards.
fn draw_cut_marks(raster: &mut Raster, at: &PixelRect, dpi_x: f64, dpi_y: f64) {
    if at.width == 0 || at.height == 0 {
        return;
    }

    let thick_x = mm_to_px_exact(CUT_MARK_THICKNESS_MM, dpi_x).round().max(1.0) as u32;
    let thick_y = mm_to_px_exact(CUT_MARK_THICKNESS_MM, dpi_y).round().max(1.0) as u32;

    // Darken towards the guide grey, never lighten: the sheet is white, so
    // blending towards white would leave nothing visible at all.
    let mark = |raster: &mut Raster, x: u32, y: u32| {
        if x >= raster.width_px || y >= raster.height_px {
            return;
        }
        let o = raster.offset(x, y);
        for c in 0..3 {
            raster.pixels[o + c] = raster.pixels[o + c].min(CUT_MARK_GREY);
        }
    };

    // The rectangle the frame occupies, one band outside the photograph on
    // every side. Saturating so a placement flush against the sheet edge
    // simply loses the part that falls off it.
    let outer_left = at.x.saturating_sub(thick_x);
    let outer_top = at.y.saturating_sub(thick_y);
    let outer_right = at.x.saturating_add(at.width).saturating_add(thick_x);
    let outer_bottom = at.y.saturating_add(at.height).saturating_add(thick_y);

    // Top and bottom bands, drawn the full width of the frame so the corners
    // are filled and the rectangle closes.
    for y in outer_top..at.y {
        for x in outer_left..outer_right {
            mark(raster, x, y);
        }
    }
    for y in at.y.saturating_add(at.height)..outer_bottom {
        for x in outer_left..outer_right {
            mark(raster, x, y);
        }
    }

    // Left and right bands, spanning only the height of the photograph: the
    // corners are already covered above.
    for y in at.y..at.y.saturating_add(at.height) {
        for x in outer_left..at.x {
            mark(raster, x, y);
        }
        for x in at.x.saturating_add(at.width)..outer_right {
            mark(raster, x, y);
        }
    }
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
            lock_orientation: false,
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
            lock_orientation: false,
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
    fn cut_marks_never_touch_the_photograph() {
        // The one thing that must not happen: a guide printed into the picture
        // cannot be trimmed off afterwards.
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(60.0, 60.0);
        let p = placement(10.0, 10.0, 20.0, 25.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let plain =
            render_sheet_with_photo(&sheet_with(p), paper, &RenderParams::new(300.0, 300.0), &source);

        let mut marked_params = RenderParams::new(300.0, 300.0);
        marked_params.cut_marks = true;
        let marked = render_sheet_with_photo(&sheet_with(p), paper, &marked_params, &source);

        assert_ne!(plain.pixels, marked.pixels, "cut marks drew nothing");

        // Every pixel inside the placement must be identical either way.
        let r = placement_to_pixels(&p, 300.0, 300.0);
        for y in r.y..r.y + r.height {
            for x in r.x..r.x + r.width {
                let o = plain.offset(x, y);
                assert_eq!(
                    plain.pixels[o..o + 4],
                    marked.pixels[o..o + 4],
                    "cut mark bled into the photo at {x},{y}"
                );
            }
        }
    }

    #[test]
    fn cut_marks_are_grey_rather_than_black() {
        // Grey so the guide does not show as a hard line on the cut edge, but
        // dark enough that halftoning still lays down ink. An earlier 210 was
        // faint on screen and printed as nothing at all.
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(60.0, 60.0);
        let p = placement(10.0, 10.0, 20.0, 25.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let mut params = RenderParams::new(300.0, 300.0);
        params.cut_marks = true;
        let marked = render_sheet_with_photo(&sheet_with(p), paper, &params, &source);

        // Sample just left of the placement's top edge, where a tick runs.
        let r = placement_to_pixels(&p, 300.0, 300.0);
        let o = marked.offset(r.x - 2, r.y);
        let v = marked.pixels[o];
        assert!(v < 250, "mark is invisible: {v}");
        assert!(v > 128, "mark is darker than a faint guide should be: {v}");
    }

    /// The guide is a closed frame around the photograph, nothing more.
    ///
    /// Lines drawn across the whole sheet carried on through the empty part of
    /// the paper and left a grid of cells where no photograph was. The frame
    /// must stop at the picture it belongs to.
    #[test]
    fn cut_marks_frame_the_photo_without_running_across_the_sheet() {
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(60.0, 60.0);
        // Set well in from every edge, so a sheet-wide line would be obvious.
        let p = placement(20.0, 20.0, 20.0, 25.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let mut params = RenderParams::new(300.0, 300.0);
        params.cut_marks = true;
        let marked = render_sheet_with_photo(&sheet_with(p), paper, &params, &source);
        let r = placement_to_pixels(&p, 300.0, 300.0);

        // All four sides are drawn, so the rectangle closes.
        let mid_x = r.x + r.width / 2;
        let mid_y = r.y + r.height / 2;
        for (x, y, side) in [
            (mid_x, r.y - 1, "top"),
            (mid_x, r.y + r.height, "bottom"),
            (r.x - 1, mid_y, "left"),
            (r.x + r.width, mid_y, "right"),
        ] {
            let v = marked.pixels[marked.offset(x, y)];
            assert!(v < 250, "{side} of the frame is missing: {v}");
        }

        // The corners are filled, so the frame has no gaps.
        for (x, y) in [(r.x - 1, r.y - 1), (r.x + r.width, r.y + r.height)] {
            let v = marked.pixels[marked.offset(x, y)];
            assert!(v < 250, "frame corner at {x},{y} is open: {v}");
        }

        // Away from the photograph the paper stays clean: no grid, and the
        // sheet edges carry nothing at all.
        for (x, y, where_) in [
            (0u32, r.y, "left edge of the sheet"),
            (marked.width_px - 1, r.y, "right edge of the sheet"),
            (r.x, 0u32, "top edge of the sheet"),
            (r.x, marked.height_px - 1, "bottom edge of the sheet"),
        ] {
            let v = marked.pixels[marked.offset(x, y)];
            assert_eq!(v, 255, "a line ran across the sheet at the {where_}");
        }
    }

    /// The marks must have a physical width, not a pixel one.
    ///
    /// This is what made them invisible in print: a single-pixel line is
    /// 0.085mm at 300dpi and half that at 600, so the better the printer the
    /// fainter the guide, until it disappeared. Asserting in millimetres means
    /// the guide cannot silently thin out again as resolution rises.
    #[test]
    fn cut_marks_keep_their_width_in_millimetres_at_any_resolution() {
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(60.0, 60.0);
        let p = placement(10.0, 10.0, 20.0, 25.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        for dpi in [300.0, 600.0, 1200.0] {
            let mut params = RenderParams::new(dpi, dpi);
            params.cut_marks = true;
            let marked = render_sheet_with_photo(&sheet_with(p), paper, &params, &source);
            let r = placement_to_pixels(&p, dpi, dpi);

            // Walk up from the top edge, counting the marked rows of the
            // horizontal tick that runs left of the placement.
            let x = r.x - 2;
            let mut rows = 0u32;
            for dy in 0..32 {
                let y = r.y.saturating_sub(dy);
                if marked.pixels[marked.offset(x, y)] < 250 {
                    rows += 1;
                } else {
                    break;
                }
            }

            let mm = rows as f64 * 25.4 / dpi;
            assert!(
                mm >= CUT_MARK_THICKNESS_MM * 0.5,
                "at {dpi}dpi the tick is only {mm:.3}mm ({rows}px), too thin to print"
            );
        }
    }

    #[test]
    fn cut_marks_are_off_by_default() {
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(60.0, 60.0);
        let p = placement(10.0, 10.0, 20.0, 25.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let params = RenderParams::new(300.0, 300.0);
        assert!(!params.cut_marks);
        let out = render_sheet_with_photo(&sheet_with(p), paper, &params, &source);

        // The margin stays pure white when marks are off.
        let r = placement_to_pixels(&p, 300.0, 300.0);
        let o = out.offset(r.x - 2, r.y);
        assert_eq!(&out.pixels[o..o + 3], &[255, 255, 255]);
    }

    #[test]
    fn turning_the_photo_leaves_the_frame_alone() {
        // The distinction that took two attempts to get right: the frame's
        // shape comes from the layout, the picture's orientation from this
        // flag. Turning the photo must not resize the rectangle it sits in.
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(60.0, 60.0);
        let p = placement(5.0, 5.0, 25.0, 20.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let upright =
            render_sheet_with_photo(&sheet_with(p), paper, &RenderParams::new(300.0, 300.0), &source);

        let mut turned_params = RenderParams::new(300.0, 300.0);
        turned_params.turn_photo = true;
        let turned = render_sheet_with_photo(&sheet_with(p), paper, &turned_params, &source);

        assert_ne!(upright.pixels, turned.pixels, "turn_photo changed nothing");
        assert_eq!(
            (upright.width_px, upright.height_px),
            (turned.width_px, turned.height_px),
            "turning the photo must not change the sheet"
        );

        // Upright the placement's top-left is the crop's top-left (red);
        // turned clockwise it becomes the crop's bottom-left (blue).
        let r = placement_to_pixels(&p, 300.0, 300.0);
        let up = &upright.pixels[upright.offset(r.x + 4, r.y + 4)..][..4];
        assert!(up[2] > 200 && up[1] < 60, "upright corner is not red: {up:?}");
        let tn = &turned.pixels[turned.offset(r.x + 4, r.y + 4)..][..4];
        assert!(tn[0] > 200 && tn[2] < 60, "turned corner is not blue: {tn:?}");
    }

    #[test]
    fn turning_a_photo_in_a_rotated_frame_puts_it_upright() {
        // A landscape placement already turns the picture, so asking for a
        // turn on top of that must cancel rather than turn twice.
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(60.0, 60.0);
        let landscape = Placement {
            x_mm: 5.0,
            y_mm: 5.0,
            size: SizeMm::new(25.0, 20.0),
            orientation: Orientation::Landscape,
        };
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let mut params = RenderParams::new(300.0, 300.0);
        params.turn_photo = true;
        let out = render_sheet_with_photo(&sheet_with(landscape), paper, &params, &source);

        let r = placement_to_pixels(&landscape, 300.0, 300.0);
        let c = &out.pixels[out.offset(r.x + 4, r.y + 4)..][..4];
        assert!(c[2] > 200 && c[1] < 60, "expected red (upright), got {c:?}");
    }

    #[test]
    fn a_mixed_sheet_draws_every_size_at_its_own_size() {
        // The bug this guards against: reusing one resampled bitmap for every
        // placement, which would print the small format stretched to the large
        // one's pixels.
        use crate::layout::{solve_mixed, PhotoGroup};

        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(100.0, 150.0);
        let groups = [
            PhotoGroup { size: SizeMm::new(35.0, 45.0), count: 2 },
            PhotoGroup { size: SizeMm::new(30.0, 35.0), count: 2 },
        ];
        let mixed = solve_mixed(paper, &groups, 3.0, 2.0).unwrap();
        assert_eq!(mixed.unplaced, vec![0, 0]);

        let placements: Vec<Placement> = mixed.placements.iter().map(|g| g.placement).collect();
        let params = RenderParams::new(300.0, 300.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);
        let raster = render_mixed_sheet(&placements, paper, &params, &source);

        // Each placement must carry the photo, sampled at its own dimensions.
        for (i, p) in placements.iter().enumerate() {
            let r = placement_to_pixels(p, 300.0, 300.0);
            let o = raster.offset(r.x + 5, r.y + 5);
            let bgra = &raster.pixels[o..o + 4];
            assert!(
                bgra[2] > 200 && bgra[1] < 60,
                "placement {i} is not red at its top-left: {bgra:?}"
            );
        }

        // And the two groups must genuinely differ in size on paper.
        let sizes: std::collections::BTreeSet<(u32, u32)> = placements
            .iter()
            .map(|p| {
                let r = placement_to_pixels(p, 300.0, 300.0);
                (r.width, r.height)
            })
            .collect();
        assert_eq!(sizes.len(), 2, "expected two distinct printed sizes, got {sizes:?}");
    }

    #[test]
    fn a_mixed_sheet_with_one_size_matches_the_single_size_renderer() {
        // The two paths must agree, or the mixed mode would quietly print
        // differently from the normal mode on identical input.
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let paper = SizeMm::new(100.0, 100.0);
        let p = placement(5.0, 5.0, 20.0, 20.0);
        let params = RenderParams::new(300.0, 300.0);
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);

        let single = render_sheet_with_photo(&sheet_with(p), paper, &params, &source);
        let mixed = render_mixed_sheet(&[p], paper, &params, &source);

        assert_eq!(single.pixels, mixed.pixels, "mixed and single renderers disagree");
    }

    #[test]
    fn an_empty_mixed_sheet_stays_white() {
        let px = quadrant_image();
        let src = ImageRef::new(&px, 100, 100).unwrap();
        let source = PhotoSource::new(src, 0.0, 0.0, 100.0, 100.0);
        let raster = render_mixed_sheet(
            &[],
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
