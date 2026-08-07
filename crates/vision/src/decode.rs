//! Turning YuNet's raw output into face detections.
//!
//! Kept free of ONNX Runtime so the arithmetic that actually decides where a
//! face lands can be tested without a model or a GPU. The model produces
//! per-anchor offsets over three feature-map strides; this maps them back to
//! image coordinates and suppresses duplicates.

use domain::geometry::{Detection, FaceLandmarks, Point, Rect};

/// Strides of YuNet's three detection heads.
pub const STRIDES: [u32; 3] = [8, 16, 32];

/// How the source image was fitted into the model's fixed input.
///
/// Detections come back in letterboxed coordinates and have to be mapped
/// through this to reach source pixels; forgetting the offset is the classic
/// way to get boxes that are subtly off-centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letterbox {
    pub scale: f64,
    pub pad_x: f64,
    pub pad_y: f64,
    pub input_width: u32,
    pub input_height: u32,
}

impl Letterbox {
    /// Fit `src` into `dst` preserving aspect ratio, centring the result.
    pub fn fit(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Self {
        let scale = (dst_w as f64 / src_w as f64).min(dst_h as f64 / src_h as f64);
        let scaled_w = src_w as f64 * scale;
        let scaled_h = src_h as f64 * scale;
        Self {
            scale,
            pad_x: (dst_w as f64 - scaled_w) / 2.0,
            pad_y: (dst_h as f64 - scaled_h) / 2.0,
            input_width: dst_w,
            input_height: dst_h,
        }
    }

    /// Map a point from model input space back to source-image pixels.
    pub fn to_source(&self, x: f64, y: f64) -> Point {
        Point::new((x - self.pad_x) / self.scale, (y - self.pad_y) / self.scale)
    }
}

/// Compute the letterbox for fitting a source image into the model input.
pub fn letterbox(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Letterbox {
    Letterbox::fit(src_w, src_h, dst_w, dst_h)
}

#[derive(Debug, Clone, Copy)]
pub struct DecodeParams {
    pub score_threshold: f32,
    pub nms_iou_threshold: f32,
    pub top_k: usize,
}

impl Default for DecodeParams {
    fn default() -> Self {
        // 0.6 keeps a clearly visible face while rejecting texture that merely
        // resembles one; a document photo has exactly one subject, so recall
        // matters less than avoiding a spurious second detection.
        Self { score_threshold: 0.6, nms_iou_threshold: 0.3, top_k: 50 }
    }
}

/// One anchor's raw prediction, as laid out by YuNet's output tensors.
///
/// The model has twelve outputs: `cls_N`, `obj_N`, `bbox_N` and `kps_N` for
/// each of the three strides. Confidence is the product of the classification
/// and objectness scores, which is why `score` is built rather than read.
#[derive(Debug, Clone, Copy)]
pub struct RawAnchor {
    pub score: f32,
    /// Box offsets: dx, dy, log-width, log-height.
    pub bbox: [f32; 4],
    /// Five landmark offsets, x then y.
    pub landmarks: [f32; 10],
    pub stride: u32,
    /// Anchor position on the feature map.
    pub col: u32,
    pub row: u32,
}

/// Combine YuNet's two score tensors into one confidence.
///
/// `cls` is how face-like the region looks, `obj` how likely it is to contain
/// an object at all. The model is trained with these multiplied; using either
/// alone produces far too many detections.
pub fn combined_score(cls: f32, obj: f32) -> f32 {
    (cls.clamp(0.0, 1.0) * obj.clamp(0.0, 1.0)).sqrt()
}

/// Decode one anchor into image-space coordinates.
///
/// YuNet predicts offsets relative to the anchor's cell centre, with the box
/// size in log space.
fn decode_anchor(a: &RawAnchor, lb: &Letterbox) -> Detection {
    let s = a.stride as f64;
    let cx = (a.col as f64 + a.bbox[0] as f64) * s;
    let cy = (a.row as f64 + a.bbox[1] as f64) * s;
    let w = (a.bbox[2] as f64).exp() * s;
    let h = (a.bbox[3] as f64).exp() * s;

    let top_left = lb.to_source(cx - w / 2.0, cy - h / 2.0);
    let bottom_right = lb.to_source(cx + w / 2.0, cy + h / 2.0);

    let lm = |i: usize| -> Point {
        let x = (a.col as f64 + a.landmarks[i * 2] as f64) * s;
        let y = (a.row as f64 + a.landmarks[i * 2 + 1] as f64) * s;
        lb.to_source(x, y)
    };

    Detection {
        bbox: Rect::new(
            top_left.x,
            top_left.y,
            bottom_right.x - top_left.x,
            bottom_right.y - top_left.y,
        ),
        landmarks: FaceLandmarks {
            right_eye: lm(0),
            left_eye: lm(1),
            nose: lm(2),
            mouth_right: lm(3),
            mouth_left: lm(4),
        },
        confidence: a.score,
    }
}

fn iou(a: &Rect, b: &Rect) -> f64 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = a.right().min(b.right());
    let y2 = a.bottom().min(b.bottom());

    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    if inter <= 0.0 {
        return 0.0;
    }
    let union = a.width * a.height + b.width * b.height - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Greedy non-maximum suppression, highest score first.
pub fn non_max_suppression(mut dets: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
    dets.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let mut kept: Vec<Detection> = Vec::new();
    for d in dets {
        if kept.iter().all(|k| iou(&k.bbox, &d.bbox) <= iou_threshold as f64) {
            kept.push(d);
        }
    }
    kept
}

/// Decode raw anchors into detections, sorted best first.
pub fn decode_yunet(
    anchors: &[RawAnchor],
    lb: &Letterbox,
    params: &DecodeParams,
) -> Vec<Detection> {
    let candidates: Vec<Detection> = anchors
        .iter()
        .filter(|a| a.score >= params.score_threshold)
        .map(|a| decode_anchor(a, lb))
        .filter(|d| d.bbox.width > 1.0 && d.bbox.height > 1.0)
        .collect();

    let mut kept = non_max_suppression(candidates, params.nms_iou_threshold);
    kept.truncate(params.top_k);
    kept
}

/// Pick the face a document photo is about: the largest, not the most
/// confident. A confident background face is still not the subject.
pub fn primary_face(dets: &[Detection]) -> Option<&Detection> {
    dets.iter().max_by(|a, b| {
        (a.bbox.width * a.bbox.height).total_cmp(&(b.bbox.width * b.bbox.height))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(score: f32, col: u32, row: u32, stride: u32) -> RawAnchor {
        RawAnchor {
            score,
            // Centred in the cell, box exactly one stride across (log(1) = 0).
            bbox: [0.5, 0.5, 0.0, 0.0],
            landmarks: [0.3, 0.4, 0.7, 0.4, 0.5, 0.6, 0.38, 0.8, 0.62, 0.8],
            stride,
            col,
            row,
        }
    }

    #[test]
    fn letterbox_centres_a_wide_image() {
        // 640x480 into 320x320: scale 0.5, so 320x240 centred vertically.
        let lb = letterbox(640, 480, 320, 320);
        assert!((lb.scale - 0.5).abs() < 1e-9);
        assert!((lb.pad_x - 0.0).abs() < 1e-9);
        assert!((lb.pad_y - 40.0).abs() < 1e-9);
    }

    #[test]
    fn letterbox_round_trips_a_point() {
        let lb = letterbox(1280, 960, 320, 320);
        // A point at the centre of the source maps to the centre of the input.
        let centre = lb.to_source(160.0, 160.0);
        assert!((centre.x - 640.0).abs() < 1e-6);
        assert!((centre.y - 480.0).abs() < 1e-6);
    }

    #[test]
    fn square_image_needs_no_padding() {
        let lb = letterbox(500, 500, 320, 320);
        assert!((lb.pad_x).abs() < 1e-9);
        assert!((lb.pad_y).abs() < 1e-9);
    }

    #[test]
    fn anchor_decodes_to_the_expected_box() {
        // Identity letterbox so model space equals source space.
        let lb = letterbox(320, 320, 320, 320);
        let a = anchor(0.9, 10, 12, 8);
        let d = decode_anchor(&a, &lb);

        // Centre at (10.5, 12.5) * 8 = (84, 100); box is 8x8.
        assert!((d.bbox.center().x - 84.0).abs() < 1e-6);
        assert!((d.bbox.center().y - 100.0).abs() < 1e-6);
        assert!((d.bbox.width - 8.0).abs() < 1e-6);
    }

    #[test]
    fn landmarks_land_inside_the_box() {
        let lb = letterbox(320, 320, 320, 320);
        let d = decode_anchor(&anchor(0.9, 20, 20, 16), &lb);
        for p in [
            d.landmarks.right_eye,
            d.landmarks.left_eye,
            d.landmarks.nose,
            d.landmarks.mouth_right,
            d.landmarks.mouth_left,
        ] {
            assert!(p.x >= d.bbox.x - 1.0 && p.x <= d.bbox.right() + 1.0, "{p:?}");
            assert!(p.y >= d.bbox.y - 1.0 && p.y <= d.bbox.bottom() + 1.0, "{p:?}");
        }
        // Right eye must be left of the left eye in image coordinates.
        assert!(d.landmarks.right_eye.x < d.landmarks.left_eye.x);
    }

    #[test]
    fn low_scoring_anchors_are_dropped() {
        let lb = letterbox(320, 320, 320, 320);
        let anchors = vec![anchor(0.9, 5, 5, 8), anchor(0.1, 20, 20, 8)];
        let out = decode_yunet(&anchors, &lb, &DecodeParams::default());
        assert_eq!(out.len(), 1);
        assert!((out[0].confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn nms_removes_overlapping_duplicates() {
        let lb = letterbox(320, 320, 320, 320);
        // Two anchors on adjacent cells produce heavily overlapping boxes.
        let anchors = vec![anchor(0.95, 10, 10, 32), anchor(0.85, 10, 10, 32)];
        let out = decode_yunet(&anchors, &lb, &DecodeParams::default());
        assert_eq!(out.len(), 1, "duplicates were not suppressed");
        assert!((out[0].confidence - 0.95).abs() < 1e-6, "kept the weaker box");
    }

    #[test]
    fn distant_faces_both_survive_nms() {
        let lb = letterbox(320, 320, 320, 320);
        let anchors = vec![anchor(0.9, 2, 2, 8), anchor(0.9, 35, 35, 8)];
        let out = decode_yunet(&anchors, &lb, &DecodeParams::default());
        assert_eq!(out.len(), 2, "non-overlapping faces must not suppress each other");
    }

    #[test]
    fn combined_score_needs_both_tensors_high() {
        // A region that looks face-like but is not an object, or vice versa,
        // must not pass. Only agreement produces a high score.
        assert!(combined_score(0.99, 0.99) > 0.95);
        assert!(combined_score(0.99, 0.01) < 0.2, "objectness was ignored");
        assert!(combined_score(0.01, 0.99) < 0.2, "classification was ignored");
    }

    #[test]
    fn combined_score_clamps_out_of_range_values() {
        // Guards against a model or dequantisation quirk producing values
        // outside 0..1, which would otherwise yield a NaN from sqrt.
        assert!(combined_score(1.5, 1.5).is_finite());
        assert!(combined_score(-0.5, 0.9).is_finite());
        assert_eq!(combined_score(-0.5, 0.9), 0.0);
    }

    #[test]
    fn iou_is_one_for_identical_boxes() {
        let r = Rect::new(10.0, 10.0, 50.0, 50.0);
        assert!((iou(&r, &r) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn iou_is_zero_for_disjoint_boxes() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(100.0, 100.0, 10.0, 10.0);
        assert_eq!(iou(&a, &b), 0.0);
    }

    #[test]
    fn primary_face_is_the_largest_not_the_most_confident() {
        // A confident face in the background is still not the subject.
        let lb = letterbox(320, 320, 320, 320);
        let mut small = decode_anchor(&anchor(0.99, 5, 5, 8), &lb);
        small.confidence = 0.99;
        let mut large = decode_anchor(&anchor(0.70, 5, 5, 32), &lb);
        large.confidence = 0.70;

        let dets = vec![small, large];
        let picked = primary_face(&dets).unwrap();
        assert!((picked.bbox.width - 32.0).abs() < 1e-6, "picked the small face");
    }

    #[test]
    fn detections_map_back_through_the_letterbox() {
        // A 1280x960 source fitted into 320x320 has vertical padding; a box at
        // the input centre must land at the source centre, not 480px off.
        let lb = letterbox(1280, 960, 320, 320);
        let a = RawAnchor {
            score: 0.9,
            bbox: [0.5, 0.5, 0.0, 0.0],
            landmarks: [0.5; 10],
            stride: 32,
            col: 4,
            row: 4,
        };
        let d = decode_anchor(&a, &lb);
        // Cell centre (4.5, 4.5) * 32 = (144, 144) in input space.
        let expected = lb.to_source(144.0, 144.0);
        assert!((d.bbox.center().x - expected.x).abs() < 1e-6);
        assert!((d.bbox.center().y - expected.y).abs() < 1e-6);
        // And it must be inside the source image.
        assert!(d.bbox.center().y >= 0.0 && d.bbox.center().y <= 960.0);
    }
}
