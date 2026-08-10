//! Background masks and the edits applied to them.
//!
//! The mask is never stored as an edited bitmap. Automatic output and user
//! brush strokes are kept apart, exactly as detection and overrides are, so a
//! re-run of the model does not discard hand corrections and undo costs
//! nothing but dropping the last stroke.

use serde::{Deserialize, Serialize};

/// A single-channel coverage map: 255 is fully subject, 0 fully background.
#[derive(Debug, Clone, PartialEq)]
pub struct AlphaMask {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl AlphaMask {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        if data.len() != width as usize * height as usize {
            return None;
        }
        Some(Self { width, height, data })
    }

    pub fn filled(width: u32, height: u32, value: u8) -> Self {
        Self { width, height, data: vec![value; width as usize * height as usize] }
    }

    pub fn at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.data[y as usize * self.width as usize + x as usize]
    }

    /// Sample the mask at a fractional position, interpolating between texels.
    ///
    /// The model emits 320x320 while a printed photo is thousands of pixels
    /// across, so the mask is magnified around six times. Picking the nearest
    /// texel makes that magnification visible as blocky stair-steps along the
    /// hair and chin; interpolating turns the same data into a smooth edge.
    ///
    /// `u` and `v` are in mask pixel space, where 0.5 is the centre of the
    /// first texel.
    pub fn sample_bilinear(&self, u: f32, v: f32) -> u8 {
        if self.width == 0 || self.height == 0 {
            return 0;
        }

        // Shift to texel centres, then clamp so edges repeat instead of fading
        // to zero — a mask that faded at the border would ring the subject with
        // background colour.
        let x = (u - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v - 0.5).clamp(0.0, (self.height - 1) as f32);

        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;

        let top = self.at(x0, y0) as f32 * (1.0 - fx) + self.at(x1, y0) as f32 * fx;
        let bottom = self.at(x0, y1) as f32 * (1.0 - fx) + self.at(x1, y1) as f32 * fx;
        (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
    }
}

/// What a brush stroke does to the mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrushMode {
    /// Paint subject back in, where the model cut too much away.
    Keep,
    /// Erase to background, where the model kept too much.
    Erase,
}

/// One brush stroke, in mask coordinates.
///
/// Stored as a path rather than as painted pixels: reproducible, cheap to undo,
/// and still meaningful if the underlying mask is regenerated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrushStroke {
    pub mode: BrushMode,
    pub radius: f32,
    /// Softness of the edge, 0 for a hard circle.
    pub feather: f32,
    pub points: Vec<(f32, f32)>,
}

impl BrushStroke {
    pub fn new(mode: BrushMode, radius: f32, feather: f32) -> Self {
        Self { mode, radius: radius.max(0.5), feather: feather.max(0.0), points: Vec::new() }
    }

    pub fn push(&mut self, x: f32, y: f32) {
        self.points.push((x, y));
    }
}

/// The list of strokes applied on top of an automatic mask.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MaskEdits {
    pub strokes: Vec<BrushStroke>,
}

impl MaskEdits {
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    pub fn push(&mut self, stroke: BrushStroke) {
        self.strokes.push(stroke);
    }

    /// Remove the most recent stroke. This is the whole undo implementation.
    pub fn undo(&mut self) -> Option<BrushStroke> {
        self.strokes.pop()
    }

    pub fn clear(&mut self) {
        self.strokes.clear();
    }
}

/// Distance from a point to a line segment, used so a stroke is a capsule
/// rather than a string of disconnected dots.
fn distance_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-8 {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Coverage of a stroke at one pixel: 1.0 at the centre, falling to 0 across
/// the feather band.
fn stroke_coverage(stroke: &BrushStroke, x: f32, y: f32) -> f32 {
    let mut nearest = f32::MAX;
    match stroke.points.len() {
        0 => return 0.0,
        1 => {
            let (ax, ay) = stroke.points[0];
            nearest = ((x - ax).powi(2) + (y - ay).powi(2)).sqrt();
        }
        _ => {
            for pair in stroke.points.windows(2) {
                let d = distance_to_segment(x, y, pair[0].0, pair[0].1, pair[1].0, pair[1].1);
                if d < nearest {
                    nearest = d;
                }
            }
        }
    }

    if nearest <= stroke.radius {
        1.0
    } else if stroke.feather > 0.0 && nearest <= stroke.radius + stroke.feather {
        1.0 - (nearest - stroke.radius) / stroke.feather
    } else {
        0.0
    }
}

/// Push mask values away from the middle, tightening or loosening the edge.
///
/// `threshold` is the coverage treated as the boundary: raising it trims a
/// halo of background that the model left attached, lowering it recovers hair
/// the model cut away. Applied before brush strokes so a stroke always wins.
pub fn apply_threshold(mask: &AlphaMask, threshold: u8) -> AlphaMask {
    // 128 is the neutral point, where the model's own decision stands.
    if threshold == 128 {
        return mask.clone();
    }
    let t = threshold as f32 / 255.0;
    let data = mask
        .data
        .iter()
        .map(|&v| {
            let x = v as f32 / 255.0;
            // Remap so `t` lands at 0.5, keeping a soft edge either side rather
            // than collapsing to a hard cut.
            let y = if x < t {
                if t > 0.0 { 0.5 * x / t } else { 0.0 }
            } else if t < 1.0 {
                0.5 + 0.5 * (x - t) / (1.0 - t)
            } else {
                1.0
            };
            (y * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect();
    AlphaMask { width: mask.width, height: mask.height, data }
}

/// Apply edits to an automatic mask, producing the mask actually used.
///
/// The input is never modified: re-running the model replaces it and the
/// strokes reapply unchanged.
pub fn apply_edits(auto: &AlphaMask, edits: &MaskEdits) -> AlphaMask {
    if edits.is_empty() {
        return auto.clone();
    }

    let mut out = auto.clone();
    for stroke in &edits.strokes {
        // Only visit pixels the stroke can reach.
        let reach = stroke.radius + stroke.feather;
        let (min_x, min_y, max_x, max_y) = bounds(stroke, reach, auto.width, auto.height);

        for y in min_y..max_y {
            for x in min_x..max_x {
                let coverage = stroke_coverage(stroke, x as f32 + 0.5, y as f32 + 0.5);
                if coverage <= 0.0 {
                    continue;
                }
                let idx = y as usize * out.width as usize + x as usize;
                let current = out.data[idx] as f32;
                let target = match stroke.mode {
                    BrushMode::Keep => 255.0,
                    BrushMode::Erase => 0.0,
                };
                out.data[idx] = (current + (target - current) * coverage).round() as u8;
            }
        }
    }
    out
}

/// Pixel bounds a stroke can affect, clamped to the mask.
fn bounds(stroke: &BrushStroke, reach: f32, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for &(x, y) in &stroke.points {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if min_x > max_x {
        return (0, 0, 0, 0);
    }
    (
        (min_x - reach).floor().max(0.0) as u32,
        (min_y - reach).floor().max(0.0) as u32,
        ((max_x + reach).ceil() as i64 + 1).clamp(0, width as i64) as u32,
        ((max_y + reach).ceil() as i64 + 1).clamp(0, height as i64) as u32,
    )
}

/// Replace the background with a flat colour, using the mask as coverage.
///
/// `rgba` is modified in place. The mask is normally much smaller than the
/// image — the model works at a fixed resolution well below print size — so it
/// is sampled bilinearly. Nearest neighbour here would show the magnification
/// as stair-steps along the hair and chin.
pub fn composite_background(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    mask: &AlphaMask,
    background: [u8; 3],
) {
    if mask.width == 0 || mask.height == 0 || width == 0 || height == 0 {
        return;
    }
    let sx = mask.width as f32 / width as f32;
    let sy = mask.height as f32 / height as f32;

    for y in 0..height {
        // Sample from the centre of each destination pixel, not its corner.
        let v = (y as f32 + 0.5) * sy;
        for x in 0..width {
            let u = (x as f32 + 0.5) * sx;
            let alpha = mask.sample_bilinear(u, v) as f32 / 255.0;
            let idx = (y as usize * width as usize + x as usize) * 4;
            for c in 0..3 {
                let subject = rgba[idx + c] as f32;
                let bg = background[c] as f32;
                rgba[idx + c] = (bg + (subject - bg) * alpha).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke_at(mode: BrushMode, radius: f32, pts: &[(f32, f32)]) -> BrushStroke {
        let mut s = BrushStroke::new(mode, radius, 0.0);
        for &(x, y) in pts {
            s.push(x, y);
        }
        s
    }

    #[test]
    fn no_edits_leaves_the_mask_untouched() {
        let auto = AlphaMask::filled(16, 16, 128);
        let out = apply_edits(&auto, &MaskEdits::default());
        assert_eq!(out, auto);
    }

    #[test]
    fn a_keep_stroke_raises_coverage_to_full() {
        let auto = AlphaMask::filled(32, 32, 0);
        let mut edits = MaskEdits::default();
        edits.push(stroke_at(BrushMode::Keep, 4.0, &[(16.0, 16.0)]));

        let out = apply_edits(&auto, &edits);
        assert_eq!(out.at(16, 16), 255, "centre of the stroke was not filled");
        assert_eq!(out.at(0, 0), 0, "the stroke leaked across the whole mask");
    }

    #[test]
    fn an_erase_stroke_clears_coverage() {
        let auto = AlphaMask::filled(32, 32, 255);
        let mut edits = MaskEdits::default();
        edits.push(stroke_at(BrushMode::Erase, 4.0, &[(16.0, 16.0)]));

        let out = apply_edits(&auto, &edits);
        assert_eq!(out.at(16, 16), 0);
        assert_eq!(out.at(0, 0), 255);
    }

    #[test]
    fn a_dragged_stroke_is_continuous() {
        // Points arrive sampled from pointer events, so the gap between them
        // must be filled or the stroke comes out as dots.
        let auto = AlphaMask::filled(64, 64, 0);
        let mut edits = MaskEdits::default();
        edits.push(stroke_at(BrushMode::Keep, 3.0, &[(10.0, 32.0), (50.0, 32.0)]));

        let out = apply_edits(&auto, &edits);
        for x in [12, 20, 30, 40, 48] {
            assert_eq!(out.at(x, 32), 255, "gap in the stroke at x={x}");
        }
    }

    #[test]
    fn feather_produces_a_soft_edge() {
        let auto = AlphaMask::filled(64, 64, 0);
        let mut s = BrushStroke::new(BrushMode::Keep, 6.0, 6.0);
        s.push(32.0, 32.0);
        let mut edits = MaskEdits::default();
        edits.push(s);

        let out = apply_edits(&auto, &edits);
        assert_eq!(out.at(32, 32), 255, "centre should be solid");
        let mid = out.at(32, 41);
        assert!(mid > 0 && mid < 255, "expected a partial value, got {mid}");
        assert_eq!(out.at(32, 50), 0, "beyond the feather should be untouched");
    }

    #[test]
    fn undo_removes_only_the_last_stroke() {
        let mut edits = MaskEdits::default();
        edits.push(stroke_at(BrushMode::Keep, 2.0, &[(1.0, 1.0)]));
        edits.push(stroke_at(BrushMode::Erase, 2.0, &[(5.0, 5.0)]));

        let undone = edits.undo().expect("nothing to undo");
        assert_eq!(undone.mode, BrushMode::Erase);
        assert_eq!(edits.strokes.len(), 1);
        assert_eq!(edits.strokes[0].mode, BrushMode::Keep);
    }

    #[test]
    fn edits_survive_a_new_automatic_mask() {
        // The point of keeping strokes separate: swapping the model output
        // must not discard hand corrections.
        let mut edits = MaskEdits::default();
        edits.push(stroke_at(BrushMode::Keep, 5.0, &[(20.0, 20.0)]));

        let first = apply_edits(&AlphaMask::filled(40, 40, 0), &edits);
        let second = apply_edits(&AlphaMask::filled(40, 40, 100), &edits);

        assert_eq!(first.at(20, 20), 255);
        assert_eq!(second.at(20, 20), 255, "stroke lost when the mask changed");
        // Away from the stroke the two differ, proving the base really changed.
        assert_ne!(first.at(0, 0), second.at(0, 0));
    }

    #[test]
    fn strokes_round_trip_through_json() {
        let mut edits = MaskEdits::default();
        edits.push(stroke_at(BrushMode::Erase, 3.5, &[(1.0, 2.0), (3.0, 4.0)]));
        let json = serde_json::to_string(&edits).unwrap();
        let back: MaskEdits = serde_json::from_str(&json).unwrap();
        assert_eq!(back, edits);
    }

    #[test]
    fn background_replacement_uses_the_mask() {
        // Left half subject, right half background.
        let mut rgba = vec![0u8; 8 * 4 * 4];
        for i in 0..(8 * 4) {
            rgba[i * 4] = 200;
            rgba[i * 4 + 1] = 100;
            rgba[i * 4 + 2] = 50;
            rgba[i * 4 + 3] = 255;
        }
        let mut mask_data = vec![0u8; 8 * 4];
        for y in 0..4 {
            for x in 0..4 {
                mask_data[y * 8 + x] = 255;
            }
        }
        let mask = AlphaMask::new(8, 4, mask_data).unwrap();

        composite_background(&mut rgba, 8, 4, &mask, [235, 235, 235]);

        // Subject side keeps its colour.
        assert_eq!(rgba[0], 200);
        // Background side becomes the requested grey: row 0, column 6.
        let right = 6 * 4;
        assert_eq!(rgba[right], 235);
        assert_eq!(rgba[right + 1], 235);
    }

    #[test]
    fn a_smaller_mask_is_stretched_over_the_image() {
        // The model runs at a fixed size well below print resolution, so the
        // mask is routinely smaller than the photo.
        let mut rgba = vec![255u8; 16 * 16 * 4];
        let mask = AlphaMask::filled(4, 4, 0);
        composite_background(&mut rgba, 16, 16, &mask, [10, 20, 30]);
        assert_eq!(rgba[0], 10);
        let last = (15 * 16 + 15) * 4;
        assert_eq!(rgba[last + 2], 30);
    }

    #[test]
    fn magnifying_a_mask_produces_a_gradient_not_stair_steps() {
        // The visible symptom this fixes: a 320x320 mask stretched over a
        // 2000px photo showed ~6px blocks along the hair. Interpolating means
        // the transition spreads across the magnified pixels instead.
        let mut rgba = vec![255u8; 64 * 4];
        // Two texels: fully background on the left, fully subject on the right.
        let mask = AlphaMask::new(2, 1, vec![0, 255]).unwrap();
        composite_background(&mut rgba, 64, 1, &mask, [0, 0, 0]);

        let values: Vec<u8> = (0..64).map(|x| rgba[x * 4]).collect();
        let distinct: std::collections::BTreeSet<u8> = values.iter().copied().collect();
        assert!(
            distinct.len() > 8,
            "expected a gradient across the boundary, got {} distinct values",
            distinct.len()
        );

        // And it must still be monotonic: background at one end, subject at the
        // other, never brightening backwards.
        assert!(values.windows(2).all(|w| w[0] <= w[1]), "not monotonic: {values:?}");
    }

    #[test]
    fn bilinear_sampling_reproduces_texel_centres_exactly() {
        // Interpolation must not shift the mask. At a texel's own centre the
        // sample has to be that texel's value, or the whole mask drifts by half
        // a texel relative to the photo.
        let mask = AlphaMask::new(4, 1, vec![0, 90, 180, 255]).unwrap();
        for (i, expected) in [0u8, 90, 180, 255].iter().enumerate() {
            let got = mask.sample_bilinear(i as f32 + 0.5, 0.5);
            assert_eq!(got, *expected, "texel {i} centre sampled as {got}");
        }
    }

    #[test]
    fn sampling_outside_the_mask_clamps_to_the_edge() {
        // Falling to zero outside would ring the subject with background where
        // it touches the frame.
        let mask = AlphaMask::new(2, 2, vec![255, 255, 255, 255]).unwrap();
        assert_eq!(mask.sample_bilinear(-5.0, -5.0), 255);
        assert_eq!(mask.sample_bilinear(99.0, 99.0), 255);
    }

    #[test]
    fn a_uniform_mask_stays_uniform_when_magnified() {
        // Interpolation must not introduce variation that was never there.
        let mut rgba = vec![200u8; 32 * 32 * 4];
        let mask = AlphaMask::filled(4, 4, 255);
        composite_background(&mut rgba, 32, 32, &mask, [0, 0, 0]);
        assert!(rgba.chunks_exact(4).all(|p| p[0] == 200), "uniform mask varied");
    }

    #[test]
    fn neutral_threshold_changes_nothing() {
        let m = AlphaMask::new(4, 1, vec![0, 80, 180, 255]).unwrap();
        assert_eq!(apply_threshold(&m, 128), m);
    }

    #[test]
    fn a_high_threshold_trims_the_edge() {
        // Raising the threshold should pull partial coverage down, shrinking a
        // halo of background left attached to the subject.
        let m = AlphaMask::new(3, 1, vec![64, 128, 200]).unwrap();
        let tight = apply_threshold(&m, 200);
        assert!(tight.data[0] < 64, "partial coverage was not reduced");
        assert!(tight.data[1] < 128);
        // Fully opaque stays opaque.
        let solid = AlphaMask::new(1, 1, vec![255]).unwrap();
        assert_eq!(apply_threshold(&solid, 200).data[0], 255);
    }

    #[test]
    fn a_low_threshold_recovers_faint_detail() {
        // Lowering it should lift faint coverage, recovering hair the model
        // nearly discarded.
        let m = AlphaMask::new(2, 1, vec![40, 128]).unwrap();
        let loose = apply_threshold(&m, 60);
        assert!(loose.data[0] > 40, "faint detail was not recovered");
        assert!(loose.data[1] > 128);
    }

    #[test]
    fn threshold_keeps_the_extremes_intact() {
        // Whatever the setting, certain background stays background and
        // certain subject stays subject.
        let m = AlphaMask::new(2, 1, vec![0, 255]).unwrap();
        for t in [10u8, 60, 128, 200, 250] {
            let out = apply_threshold(&m, t);
            assert_eq!(out.data[0], 0, "background leaked in at threshold {t}");
            assert_eq!(out.data[1], 255, "subject was lost at threshold {t}");
        }
    }

    #[test]
    fn mismatched_mask_dimensions_are_rejected() {
        assert!(AlphaMask::new(4, 4, vec![0; 10]).is_none());
    }
}
