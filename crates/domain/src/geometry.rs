//! Face geometry: turning detected points into a crop rectangle.
//!
//! Pure arithmetic, no image data and no model. Everything here works in source
//! image pixels; conversion to millimetres happens once the crop is placed on
//! paper.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn midpoint(self, other: Self) -> Self {
        Self::new((self.x + other.x) / 2.0, (self.y + other.y) / 2.0)
    }

    pub fn distance_to(self, other: Self) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// Axis-aligned rectangle in source-image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Whether this rectangle lies entirely within `bounds`.
    pub fn fits_inside(&self, bounds: &Rect) -> bool {
        self.x >= bounds.x
            && self.y >= bounds.y
            && self.right() <= bounds.right()
            && self.bottom() <= bounds.bottom()
    }
}

/// The five points YuNet returns, in source-image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceLandmarks {
    pub right_eye: Point,
    pub left_eye: Point,
    pub nose: Point,
    pub mouth_right: Point,
    pub mouth_left: Point,
}

impl FaceLandmarks {
    pub fn eye_center(&self) -> Point {
        self.right_eye.midpoint(self.left_eye)
    }

    pub fn eye_distance(&self) -> f64 {
        self.right_eye.distance_to(self.left_eye)
    }

    /// Head tilt in degrees, positive clockwise.
    ///
    /// Derived from the eye line, which is the only reliable horizontal
    /// reference a five-point detector gives.
    pub fn roll_degrees(&self) -> f64 {
        let dx = self.left_eye.x - self.right_eye.x;
        let dy = self.left_eye.y - self.right_eye.y;
        dy.atan2(dx).to_degrees()
    }
}

/// One detected face.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub bbox: Rect,
    pub landmarks: FaceLandmarks,
    pub confidence: f32,
}

/// Vertical anchors a passport crop is built from.
///
/// A five-point detector cannot see either of these directly: the crown is
/// under the hair and the chin edge is not a landmark. Both are therefore
/// estimated from the face box and flagged as such, so the UI can mark them
/// AUTO and let the user drag them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeadAnchors {
    pub chin: Point,
    pub crown: Point,
    /// True when these came from the estimator rather than a measurement.
    pub estimated: bool,
}

/// How far below the face box the chin sits, as a fraction of box height.
///
/// DERIVED, not from any regulation: YuNet's box ends around the jaw, slightly
/// above the chin tip. Overridable in the UI.
const CHIN_BELOW_BOX_RATIO: f64 = 0.06;

/// Where the eye line falls between chin and crown, measured from the chin.
///
/// DERIVED from standard facial anthropometry: in adults the pupils sit close
/// to the midpoint of chin-to-crown height. Estimating the crown from this
/// rather than from the top of the face box matters, because the box top is a
/// detector artefact while the eye line is an actual measured landmark.
/// Overridable in the UI.
const EYE_LINE_FRACTION_OF_HEAD: f64 = 0.5;

/// Estimate chin and crown from a detection.
///
/// The crown is derived from the eye line rather than from the face box: the
/// box top varies with hair and framing, whereas the eyes are detected
/// directly. Both results are flagged `estimated` so the UI shows an AUTO
/// badge and the user can drag either point.
pub fn estimate_head_anchors(d: &Detection) -> HeadAnchors {
    let cx = d.bbox.center().x;
    let chin_y = d.bbox.bottom() + d.bbox.height * CHIN_BELOW_BOX_RATIO;
    let eye_y = d.landmarks.eye_center().y;

    // chin_to_eye is that fraction of the whole head height, so the full head
    // follows by division. Guard against a detection with eyes at or below the
    // chin, which would otherwise invert the head.
    let chin_to_eye = (chin_y - eye_y).max(1.0);
    let head_height = chin_to_eye / EYE_LINE_FRACTION_OF_HEAD;

    HeadAnchors {
        chin: Point::new(cx, chin_y),
        crown: Point::new(cx, chin_y - head_height),
        estimated: true,
    }
}

/// What a spec demands of the crop, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropTarget {
    pub photo_width_mm: f64,
    pub photo_height_mm: f64,
    /// Desired head height (chin to crown) as printed.
    pub head_height_mm: f64,
    /// Distance from the bottom edge of the photo up to the chin line.
    ///
    /// `None` centres the head vertically instead, which is what free mode and
    /// any spec without a measured chin line must do.
    pub chin_from_bottom_mm: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CropError {
    /// The required crop extends past the image edges.
    ///
    /// Carries the numbers so the UI can explain the problem and suggest a
    /// head height that would fit, rather than silently shifting the crop
    /// (which would cut into the face) or upscaling.
    CropOutsideImage {
        overflow_left_px: f64,
        overflow_top_px: f64,
        overflow_right_px: f64,
        overflow_bottom_px: f64,
        /// Smallest head height, in millimetres, whose crop fits this image.
        ///
        /// Counter-intuitively this is a *lower* bound: a larger head on paper
        /// means fewer millimetres per pixel and therefore a smaller crop, so
        /// the way out of an overflow is to let the head be bigger. If this
        /// exceeds what a spec permits, the photo simply cannot satisfy it and
        /// the subject must be re-photographed further from the camera.
        min_head_height_mm: f64,
    },
    InvalidTarget,
    /// Chin and crown coincide, so no scale can be derived.
    DegenerateHead,
}

/// Build the crop rectangle that puts the head at the requested size.
///
/// Works entirely in source pixels: the ratio between the head in pixels and
/// the head in millimetres fixes the scale, and the rest follows from the
/// photo's aspect ratio.
pub fn solve_crop(
    anchors: &HeadAnchors,
    target: &CropTarget,
    image: &Rect,
) -> Result<Rect, CropError> {
    if target.photo_width_mm <= 0.0
        || target.photo_height_mm <= 0.0
        || target.head_height_mm <= 0.0
    {
        return Err(CropError::InvalidTarget);
    }

    let head_px = (anchors.chin.y - anchors.crown.y).abs();
    if head_px < 1e-6 {
        return Err(CropError::DegenerateHead);
    }

    // Pixels per millimetre implied by the requested head height.
    let px_per_mm = head_px / target.head_height_mm;

    let crop_w = target.photo_width_mm * px_per_mm;
    let crop_h = target.photo_height_mm * px_per_mm;

    // Horizontal: centre on the head.
    let cx = (anchors.chin.x + anchors.crown.x) / 2.0;
    let x = cx - crop_w / 2.0;

    // Vertical: either pin the chin at the requested height above the bottom
    // edge, or centre the head in the frame.
    let y = match target.chin_from_bottom_mm {
        Some(mm) => anchors.chin.y + mm * px_per_mm - crop_h,
        None => {
            let head_center_y = (anchors.chin.y + anchors.crown.y) / 2.0;
            head_center_y - crop_h / 2.0
        }
    };

    let crop = Rect::new(x, y, crop_w, crop_h);
    if !crop.fits_inside(image) {
        let (l, t, r, b) = crop_overflow(&crop, image);
        return Err(CropError::CropOutsideImage {
            overflow_left_px: l,
            overflow_top_px: t,
            overflow_right_px: r,
            overflow_bottom_px: b,
            min_head_height_mm: min_head_height_mm(anchors, head_px, target, image),
        });
    }
    Ok(crop)
}

/// Smallest head height, in millimetres, whose crop still fits the image.
///
/// Two things make this less obvious than it looks. First, the crop is
/// positioned around the head, so what limits it is the room on each side of
/// the head, not the image's total size: a head near the top edge is
/// constrained by that edge however large the image is. Second, the bound is a
/// minimum rather than a maximum, because a larger head on paper produces a
/// smaller crop in pixels.
fn min_head_height_mm(
    anchors: &HeadAnchors,
    head_px: f64,
    target: &CropTarget,
    image: &Rect,
) -> f64 {
    let cx = (anchors.chin.x + anchors.crown.x) / 2.0;

    // Horizontal: the crop is centred on the head, so the usable half-width is
    // whichever side is tighter, doubled.
    let half_w = (cx - image.x).min(image.right() - cx);
    let max_crop_w = (half_w * 2.0).max(0.0);

    // Vertical: with a fixed chin line the crop hangs from the chin, otherwise
    // it is centred on the head. Each case has a different limit.
    let max_crop_h = match target.chin_from_bottom_mm {
        Some(mm) => {
            // Below the chin the crop needs `mm` worth of space; above it the
            // rest. Solve for the crop height that exactly reaches whichever
            // edge is hit first.
            let below = (image.bottom() - anchors.chin.y).max(0.0);
            let above = (anchors.chin.y - image.y).max(0.0);
            let frac_below = mm / target.photo_height_mm;
            let frac_above = 1.0 - frac_below;
            let by_below = if frac_below > 0.0 { below / frac_below } else { f64::MAX };
            let by_above = if frac_above > 0.0 { above / frac_above } else { f64::MAX };
            by_below.min(by_above)
        }
        None => {
            let head_cy = (anchors.chin.y + anchors.crown.y) / 2.0;
            let half_h = (head_cy - image.y).min(image.bottom() - head_cy);
            (half_h * 2.0).max(0.0)
        }
    };

    // Convert whichever crop dimension binds into a head height.
    let by_width = max_crop_w / target.photo_width_mm;
    let by_height = max_crop_h / target.photo_height_mm;
    let max_px_per_mm = by_width.min(by_height);
    if max_px_per_mm <= 0.0 {
        return 0.0;
    }
    head_px / max_px_per_mm
}

/// How much of the requested crop falls outside the image, in pixels.
///
/// Returned so the UI can say "move the camera back" with a number rather than
/// simply refusing.
pub fn crop_overflow(crop: &Rect, image: &Rect) -> (f64, f64, f64, f64) {
    (
        (image.x - crop.x).max(0.0),
        (image.y - crop.y).max(0.0),
        (crop.right() - image.right()).max(0.0),
        (crop.bottom() - image.bottom()).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(box_x: f64, box_y: f64, w: f64, h: f64) -> Detection {
        // Eyes placed at a plausible height inside the box.
        let eye_y = box_y + h * 0.35;
        Detection {
            bbox: Rect::new(box_x, box_y, w, h),
            landmarks: FaceLandmarks {
                right_eye: Point::new(box_x + w * 0.3, eye_y),
                left_eye: Point::new(box_x + w * 0.7, eye_y),
                nose: Point::new(box_x + w * 0.5, box_y + h * 0.55),
                mouth_right: Point::new(box_x + w * 0.38, box_y + h * 0.78),
                mouth_left: Point::new(box_x + w * 0.62, box_y + h * 0.78),
            },
            confidence: 0.99,
        }
    }

    #[test]
    fn eye_distance_and_center() {
        let d = detection(100.0, 100.0, 200.0, 200.0);
        assert!((d.landmarks.eye_distance() - 80.0).abs() < 1e-9);
        assert!((d.landmarks.eye_center().x - 200.0).abs() < 1e-9);
    }

    #[test]
    fn level_eyes_mean_zero_roll() {
        let d = detection(0.0, 0.0, 100.0, 100.0);
        assert!(d.landmarks.roll_degrees().abs() < 1e-9);
    }

    #[test]
    fn tilted_eyes_produce_roll() {
        let mut d = detection(0.0, 0.0, 100.0, 100.0);
        // Raise the left eye by the same amount it is offset horizontally: 45deg.
        d.landmarks.left_eye.y -= 40.0;
        assert!((d.landmarks.roll_degrees() + 45.0).abs() < 0.01);
    }

    #[test]
    fn crown_is_above_and_chin_below_the_face_box() {
        let d = detection(100.0, 100.0, 200.0, 200.0);
        let a = estimate_head_anchors(&d);
        assert!(a.crown.y < d.bbox.y, "crown must sit above the box top");
        assert!(a.chin.y > d.bbox.bottom(), "chin must sit below the box bottom");
        assert!(a.estimated, "estimates must be flagged for the AUTO badge");
        // Both on the vertical centre line of the face.
        assert!((a.crown.x - 200.0).abs() < 1e-9);
        assert!((a.chin.x - 200.0).abs() < 1e-9);
    }

    #[test]
    fn crown_estimate_stays_inside_a_real_photo() {
        // Regression: these are the values YuNet actually returned for a
        // 2000x1333 test photo. An earlier estimator derived the crown from
        // the box top and put it at y = -6, outside the image, which made the
        // crop impossible.
        let d = Detection {
            bbox: Rect::new(804.0, 180.0, 395.0, 518.0),
            landmarks: FaceLandmarks {
                right_eye: Point::new(913.0, 379.0),
                left_eye: Point::new(1100.0, 378.0),
                nose: Point::new(1000.0, 480.0),
                mouth_right: Point::new(940.0, 580.0),
                mouth_left: Point::new(1060.0, 580.0),
            },
            confidence: 0.947,
        };
        let a = estimate_head_anchors(&d);
        assert!(a.crown.y > 0.0, "crown at y={} is off the top of the image", a.crown.y);
        assert!(a.chin.y < 1333.0, "chin at y={} is off the bottom", a.chin.y);
        // Head height should be plausible: taller than the box, not double it.
        let head = a.chin.y - a.crown.y;
        assert!(head > 518.0 && head < 900.0, "implausible head height {head}px");
    }

    #[test]
    fn crown_is_derived_from_the_eye_line_not_the_box() {
        // Two detections with the same box but different eye heights must give
        // different crowns; otherwise the eye line is being ignored.
        let base = detection(100.0, 100.0, 200.0, 200.0);
        let mut higher_eyes = base;
        higher_eyes.landmarks.right_eye.y -= 30.0;
        higher_eyes.landmarks.left_eye.y -= 30.0;

        let a = estimate_head_anchors(&base);
        let b = estimate_head_anchors(&higher_eyes);
        assert!(
            (a.crown.y - b.crown.y).abs() > 1.0,
            "eye line had no effect on the crown estimate"
        );
    }

    #[test]
    fn eyes_below_the_chin_do_not_invert_the_head() {
        // A badly wrong detection must still produce a crown above the chin.
        let mut d = detection(100.0, 100.0, 200.0, 200.0);
        d.landmarks.right_eye.y = 400.0;
        d.landmarks.left_eye.y = 400.0;
        let a = estimate_head_anchors(&d);
        assert!(a.crown.y < a.chin.y, "head is inverted");
    }

    #[test]
    fn crop_gives_the_requested_head_height() {
        // A 300px head asked to print at 33mm on a 35x45mm photo.
        let anchors = HeadAnchors {
            chin: Point::new(500.0, 700.0),
            crown: Point::new(500.0, 400.0),
            estimated: true,
        };
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.0,
            chin_from_bottom_mm: None,
        };
        let image = Rect::new(0.0, 0.0, 2000.0, 2000.0);
        let crop = solve_crop(&anchors, &target, &image).unwrap();

        // 300px / 33mm = 9.0909 px per mm.
        let px_per_mm = 300.0 / 33.0;
        assert!((crop.width - 35.0 * px_per_mm).abs() < 1e-6);
        assert!((crop.height - 45.0 * px_per_mm).abs() < 1e-6);

        // The head must occupy exactly head_height_mm of the crop's height.
        let head_fraction = 300.0 / crop.height;
        assert!((head_fraction - 33.0 / 45.0).abs() < 1e-9);
    }

    #[test]
    fn chin_line_is_honoured_when_specified() {
        let anchors = HeadAnchors {
            chin: Point::new(500.0, 700.0),
            crown: Point::new(500.0, 400.0),
            estimated: true,
        };
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.0,
            chin_from_bottom_mm: Some(7.0),
        };
        let image = Rect::new(0.0, 0.0, 2000.0, 2000.0);
        let crop = solve_crop(&anchors, &target, &image).unwrap();

        let px_per_mm = 300.0 / 33.0;
        let chin_above_bottom_px = crop.bottom() - anchors.chin.y;
        assert!(
            (chin_above_bottom_px - 7.0 * px_per_mm).abs() < 1e-6,
            "chin sits {chin_above_bottom_px}px above the bottom edge"
        );
    }

    #[test]
    fn crop_is_horizontally_centred_on_the_head() {
        let anchors = HeadAnchors {
            chin: Point::new(640.0, 700.0),
            crown: Point::new(640.0, 400.0),
            estimated: true,
        };
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.0,
            chin_from_bottom_mm: None,
        };
        let image = Rect::new(0.0, 0.0, 2000.0, 2000.0);
        let crop = solve_crop(&anchors, &target, &image).unwrap();
        assert!((crop.center().x - 640.0).abs() < 1e-6);
    }

    #[test]
    fn head_too_large_for_the_frame_is_rejected() {
        // Head fills almost the whole image, so a 45mm-tall crop cannot fit.
        let anchors = HeadAnchors {
            chin: Point::new(250.0, 480.0),
            crown: Point::new(250.0, 20.0),
            estimated: true,
        };
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.0,
            chin_from_bottom_mm: None,
        };
        let image = Rect::new(0.0, 0.0, 500.0, 500.0);
        assert!(matches!(
            solve_crop(&anchors, &target, &image),
            Err(CropError::CropOutsideImage { .. })
        ));
    }

    #[test]
    fn overflow_error_suggests_a_head_height_that_fits() {
        // Regression from the real test photo: a 701px head asked to print at
        // 33.75mm needs a 935px-tall crop, but the head sits high in a 1333px
        // image, so the crop overflows the top. The error must carry both the
        // overflow and a head height that actually works.
        let anchors = HeadAnchors {
            chin: Point::new(1001.0, 729.0),
            crown: Point::new(1001.0, 28.0),
            estimated: true,
        };
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.75,
            chin_from_bottom_mm: None,
        };
        let image = Rect::new(0.0, 0.0, 2000.0, 1333.0);

        let Err(CropError::CropOutsideImage {
            overflow_top_px,
            min_head_height_mm,
            ..
        }) = solve_crop(&anchors, &target, &image)
        else {
            panic!("expected an overflow error");
        };

        assert!(overflow_top_px > 0.0, "the overflow should be at the top");
        // A larger head means a smaller crop, so the escape route is upwards.
        assert!(
            min_head_height_mm > 33.75,
            "suggestion {min_head_height_mm}mm should exceed the request"
        );

        // The suggested height must actually fit. A hair above it, to stay
        // clear of the boundary where rounding could tip it back over.
        let retry = CropTarget {
            head_height_mm: min_head_height_mm * 1.001,
            ..target
        };
        assert!(
            solve_crop(&anchors, &retry, &image).is_ok(),
            "the suggested head height {min_head_height_mm}mm still does not fit"
        );
    }

    #[test]
    fn a_head_high_in_frame_cannot_meet_a_strict_spec() {
        // Documents the real limitation found with the test photo: when the
        // subject is framed too close, the smallest workable head height can
        // exceed what the regulation permits. The app must surface this rather
        // than produce a photo that will be rejected at the counter.
        let anchors = HeadAnchors {
            chin: Point::new(1001.0, 729.0),
            crown: Point::new(1001.0, 28.0),
            estimated: true,
        };
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.75,
            chin_from_bottom_mm: None,
        };
        let image = Rect::new(0.0, 0.0, 2000.0, 1333.0);

        let Err(CropError::CropOutsideImage { min_head_height_mm, .. }) =
            solve_crop(&anchors, &target, &image)
        else {
            panic!("expected an overflow error");
        };

        // Croatian passport permits 31.5-36mm for adults.
        assert!(
            min_head_height_mm > 36.0,
            "this photo was expected to fall outside the legal range, got {min_head_height_mm}mm"
        );
    }

    #[test]
    fn overflow_reports_how_far_outside_the_crop_reaches() {
        let crop = Rect::new(-30.0, -20.0, 200.0, 200.0);
        let image = Rect::new(0.0, 0.0, 150.0, 150.0);
        let (l, t, r, b) = crop_overflow(&crop, &image);
        assert_eq!((l, t), (30.0, 20.0));
        assert_eq!((r, b), (20.0, 30.0));
    }

    #[test]
    fn degenerate_head_is_rejected() {
        let anchors = HeadAnchors {
            chin: Point::new(100.0, 200.0),
            crown: Point::new(100.0, 200.0),
            estimated: true,
        };
        let target = CropTarget {
            photo_width_mm: 35.0,
            photo_height_mm: 45.0,
            head_height_mm: 33.0,
            chin_from_bottom_mm: None,
        };
        let image = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        assert_eq!(
            solve_crop(&anchors, &target, &image),
            Err(CropError::DegenerateHead)
        );
    }

    #[test]
    fn invalid_targets_are_rejected() {
        let anchors = HeadAnchors {
            chin: Point::new(100.0, 300.0),
            crown: Point::new(100.0, 100.0),
            estimated: true,
        };
        let image = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        for (w, h, head) in [(0.0, 45.0, 33.0), (35.0, 0.0, 33.0), (35.0, 45.0, 0.0)] {
            let target = CropTarget {
                photo_width_mm: w,
                photo_height_mm: h,
                head_height_mm: head,
                chin_from_bottom_mm: None,
            };
            assert_eq!(
                solve_crop(&anchors, &target, &image),
                Err(CropError::InvalidTarget)
            );
        }
    }
}
