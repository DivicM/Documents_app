//! Non-destructive editing: automatic values and user overrides kept apart.
//!
//! Detection produces parameters, never pixels. Anything the user changes goes
//! into `Overrides` as an `Option`, so the automatic value survives underneath
//! and a reset is just clearing the option. This is what makes every control in
//! the UI resettable and what lets a re-detection replace the automatic layer
//! without discarding the user's work.

use crate::geometry::{Detection, HeadAnchors, Point, Rect};
use serde::{Deserialize, Serialize};

/// The rule that governs every control in the UI: an override wins if present,
/// otherwise the automatic value is used.
pub fn effective<T: Clone>(auto: &T, ovr: &Option<T>) -> T {
    ovr.clone().unwrap_or_else(|| auto.clone())
}

/// User edits. Every field is optional; `None` means "use the automatic value".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Overrides {
    pub face_box: Option<Rect>,
    pub chin: Option<Point>,
    pub crown: Option<Point>,
    pub right_eye: Option<Point>,
    pub left_eye: Option<Point>,
    pub crop_rect: Option<Rect>,
    pub rotation_deg: Option<f64>,
    pub head_height_mm: Option<f64>,
}

impl Overrides {
    /// Whether the user has changed anything at all.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Clear one field by name, for the per-control reset button.
    ///
    /// Returns false for an unknown name so a UI typo surfaces instead of
    /// silently doing nothing.
    pub fn reset_field(&mut self, field: &str) -> bool {
        match field {
            "face_box" => self.face_box = None,
            "chin" => self.chin = None,
            "crown" => self.crown = None,
            "right_eye" => self.right_eye = None,
            "left_eye" => self.left_eye = None,
            "crop_rect" => self.crop_rect = None,
            "rotation_deg" => self.rotation_deg = None,
            "head_height_mm" => self.head_height_mm = None,
            _ => return false,
        }
        true
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }
}

/// Which fields are currently automatic, so the UI can show an AUTO badge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutoFlags {
    pub face_box: bool,
    pub chin: bool,
    pub crown: bool,
    pub right_eye: bool,
    pub left_eye: bool,
    pub crop_rect: bool,
    pub rotation_deg: bool,
    pub head_height_mm: bool,
}

/// Detection plus overrides, resolved on demand.
///
/// Holds the automatic layer immutably: re-running detection replaces `auto`
/// and leaves every user edit intact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceState {
    pub auto_detection: Detection,
    pub auto_anchors: HeadAnchors,
    pub overrides: Overrides,
}

impl FaceState {
    pub fn new(detection: Detection, anchors: HeadAnchors) -> Self {
        Self { auto_detection: detection, auto_anchors: anchors, overrides: Overrides::default() }
    }

    pub fn face_box(&self) -> Rect {
        effective(&self.auto_detection.bbox, &self.overrides.face_box)
    }

    pub fn chin(&self) -> Point {
        effective(&self.auto_anchors.chin, &self.overrides.chin)
    }

    pub fn crown(&self) -> Point {
        effective(&self.auto_anchors.crown, &self.overrides.crown)
    }

    pub fn right_eye(&self) -> Point {
        effective(&self.auto_detection.landmarks.right_eye, &self.overrides.right_eye)
    }

    pub fn left_eye(&self) -> Point {
        effective(&self.auto_detection.landmarks.left_eye, &self.overrides.left_eye)
    }

    /// Head tilt, from overridden eyes if the user moved them.
    pub fn rotation_deg(&self) -> f64 {
        if let Some(r) = self.overrides.rotation_deg {
            return r;
        }
        let (r, l) = (self.right_eye(), self.left_eye());
        (l.y - r.y).atan2(l.x - r.x).to_degrees()
    }

    /// Effective anchors, for feeding into the crop solver.
    pub fn anchors(&self) -> HeadAnchors {
        HeadAnchors {
            chin: self.chin(),
            crown: self.crown(),
            estimated: self.overrides.chin.is_none() && self.overrides.crown.is_none(),
        }
    }

    pub fn auto_flags(&self) -> AutoFlags {
        AutoFlags {
            face_box: self.overrides.face_box.is_none(),
            chin: self.overrides.chin.is_none(),
            crown: self.overrides.crown.is_none(),
            right_eye: self.overrides.right_eye.is_none(),
            left_eye: self.overrides.left_eye.is_none(),
            crop_rect: self.overrides.crop_rect.is_none(),
            rotation_deg: self.overrides.rotation_deg.is_none(),
            head_height_mm: self.overrides.head_height_mm.is_none(),
        }
    }

    /// Replace the automatic layer, keeping user edits.
    pub fn redetect(&mut self, detection: Detection, anchors: HeadAnchors) {
        self.auto_detection = detection;
        self.auto_anchors = anchors;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{FaceLandmarks, Rect};

    fn state() -> FaceState {
        let d = Detection {
            bbox: Rect::new(100.0, 100.0, 200.0, 200.0),
            landmarks: FaceLandmarks {
                right_eye: Point::new(160.0, 170.0),
                left_eye: Point::new(240.0, 170.0),
                nose: Point::new(200.0, 210.0),
                mouth_right: Point::new(176.0, 256.0),
                mouth_left: Point::new(224.0, 256.0),
            },
            confidence: 0.99,
        };
        let a = HeadAnchors {
            chin: Point::new(200.0, 312.0),
            crown: Point::new(200.0, 28.0),
            estimated: true,
        };
        FaceState::new(d, a)
    }

    #[test]
    fn without_overrides_everything_is_automatic() {
        let s = state();
        assert!(s.overrides.is_empty());
        let f = s.auto_flags();
        assert!(f.chin && f.crown && f.face_box && f.right_eye);
        assert_eq!(s.chin(), Point::new(200.0, 312.0));
    }

    #[test]
    fn an_override_wins_and_clears_the_auto_flag() {
        let mut s = state();
        s.overrides.chin = Some(Point::new(205.0, 320.0));
        assert_eq!(s.chin(), Point::new(205.0, 320.0));
        assert!(!s.auto_flags().chin, "AUTO badge must switch off");
        // Others stay automatic.
        assert!(s.auto_flags().crown);
    }

    #[test]
    fn reset_restores_the_automatic_value() {
        let mut s = state();
        s.overrides.chin = Some(Point::new(999.0, 999.0));
        assert!(s.overrides.reset_field("chin"));
        assert_eq!(s.chin(), Point::new(200.0, 312.0));
        assert!(s.auto_flags().chin);
    }

    #[test]
    fn resetting_an_unknown_field_is_reported() {
        let mut s = state();
        assert!(!s.overrides.reset_field("no_such_field"));
    }

    #[test]
    fn redetection_keeps_user_edits() {
        // The whole point of the split: a better detection must not discard
        // what the user carefully positioned.
        let mut s = state();
        s.overrides.chin = Some(Point::new(210.0, 330.0));

        let mut better = s.auto_detection;
        better.bbox = Rect::new(110.0, 105.0, 190.0, 195.0);
        let new_anchors = HeadAnchors {
            chin: Point::new(205.0, 315.0),
            crown: Point::new(205.0, 30.0),
            estimated: true,
        };
        s.redetect(better, new_anchors);

        // User's chin survives; the untouched crown takes the new value.
        assert_eq!(s.chin(), Point::new(210.0, 330.0));
        assert_eq!(s.crown(), Point::new(205.0, 30.0));
    }

    #[test]
    fn rotation_follows_overridden_eyes() {
        let mut s = state();
        assert!(s.rotation_deg().abs() < 1e-9);

        // Drag the left eye upwards: the head is now tilted.
        s.overrides.left_eye = Some(Point::new(240.0, 130.0));
        assert!(s.rotation_deg() < -20.0, "got {}", s.rotation_deg());
    }

    #[test]
    fn explicit_rotation_override_beats_the_eye_line() {
        let mut s = state();
        s.overrides.left_eye = Some(Point::new(240.0, 130.0));
        s.overrides.rotation_deg = Some(0.0);
        assert_eq!(s.rotation_deg(), 0.0);
    }

    #[test]
    fn anchors_report_estimated_only_while_untouched() {
        let mut s = state();
        assert!(s.anchors().estimated);
        s.overrides.chin = Some(Point::new(200.0, 315.0));
        assert!(!s.anchors().estimated);
    }

    #[test]
    fn reset_all_clears_every_override() {
        let mut s = state();
        s.overrides.chin = Some(Point::new(1.0, 2.0));
        s.overrides.rotation_deg = Some(5.0);
        s.overrides.reset_all();
        assert!(s.overrides.is_empty());
    }

    #[test]
    fn state_round_trips_through_json() {
        // The UI holds this state, so it must survive the IPC boundary.
        let mut s = state();
        s.overrides.crown = Some(Point::new(200.0, 25.0));
        let json = serde_json::to_string(&s).unwrap();
        let back: FaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
