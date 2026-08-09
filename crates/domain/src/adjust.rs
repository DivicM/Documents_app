//! Exposure, contrast and white balance.
//!
//! Pure functions over an RGBA buffer, applied before cropping and before
//! background replacement so the preview and the print see identical pixels.
//!
//! Work is done in floating point and quantised back to 8-bit exactly once, at
//! the end of the chain, which is what keeps successive adjustments from
//! compounding rounding error.

use serde::{Deserialize, Serialize};

/// All adjustments, neutral by default.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Adjustments {
    /// Stops of exposure. Each +1 doubles the light.
    pub exposure_ev: f64,
    /// -1 flattens towards mid grey, +1 steepens. 0 is unchanged.
    pub contrast: f64,
    /// White balance along blue/amber. Negative cools, positive warms.
    pub temperature: f64,
    /// White balance along green/magenta.
    pub tint: f64,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

impl Adjustments {
    pub const NEUTRAL: Self =
        Self { exposure_ev: 0.0, contrast: 0.0, temperature: 0.0, tint: 0.0 };

    /// Whether applying these would change anything.
    ///
    /// Checked so an untouched photo skips the whole pass rather than being
    /// rewritten byte for byte.
    pub fn is_neutral(&self) -> bool {
        self.exposure_ev == 0.0
            && self.contrast == 0.0
            && self.temperature == 0.0
            && self.tint == 0.0
    }

    /// Clamp to ranges that stay useful; beyond these the image is destroyed
    /// rather than corrected.
    fn sanitised(self) -> Self {
        let finite = |v: f64, limit: f64| if v.is_finite() { v.clamp(-limit, limit) } else { 0.0 };
        Self {
            exposure_ev: finite(self.exposure_ev, 3.0),
            contrast: finite(self.contrast, 1.0),
            temperature: finite(self.temperature, 1.0),
            tint: finite(self.tint, 1.0),
        }
    }
}

/// Per-channel multipliers for a white balance shift.
///
/// Approximates the direction of a colour temperature change rather than doing
/// a full chromatic adaptation: for correcting a slightly warm or cool
/// portrait, which is all this is for, the difference is not visible.
fn white_balance_gains(temperature: f64, tint: f64) -> [f64; 3] {
    // 0.3 keeps the extremes of the slider usable rather than lurid.
    let t = temperature * 0.3;
    let g = tint * 0.3;
    [
        1.0 + t,        // red rises as it warms
        1.0 + g,        // green carries the tint axis
        1.0 - t,        // blue falls as it warms
    ]
}

/// Apply adjustments to an RGBA buffer in place. Alpha is untouched.
pub fn apply_adjustments(rgba: &mut [u8], adj: &Adjustments) {
    let adj = adj.sanitised();
    if adj.is_neutral() {
        return;
    }

    let exposure = 2f64.powf(adj.exposure_ev);
    let gains = white_balance_gains(adj.temperature, adj.tint);
    // Contrast pivots around mid grey so the overall brightness holds.
    let contrast = 1.0 + adj.contrast;

    // A lookup table per channel: the transform depends only on the input
    // value, so 768 evaluations replace one per pixel.
    let mut lut = [[0u8; 256]; 3];
    for (c, table) in lut.iter_mut().enumerate() {
        for (v, out) in table.iter_mut().enumerate() {
            let mut x = v as f64 / 255.0;
            x *= exposure * gains[c];
            x = 0.5 + (x - 0.5) * contrast;
            *out = (x * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    for px in rgba.chunks_exact_mut(4) {
        px[0] = lut[0][px[0] as usize];
        px[1] = lut[1][px[1] as usize];
        px[2] = lut[2][px[2] as usize];
    }
}

/// Fraction of pixels crushed to black and blown to white.
///
/// Feeds the `exposure` rule: a little clipping is normal, a lot means detail
/// is gone and no adjustment will bring it back.
pub fn clipping(rgba: &[u8]) -> (f64, f64) {
    if rgba.len() < 4 {
        return (0.0, 0.0);
    }
    let mut shadows = 0usize;
    let mut highlights = 0usize;
    let mut total = 0usize;

    for px in rgba.chunks_exact(4) {
        // Rec. 601 luma, adequate for judging exposure.
        let luma = 0.299 * px[0] as f64 + 0.587 * px[1] as f64 + 0.114 * px[2] as f64;
        if luma <= 2.0 {
            shadows += 1;
        } else if luma >= 253.0 {
            highlights += 1;
        }
        total += 1;
    }

    if total == 0 {
        return (0.0, 0.0);
    }
    (shadows as f64 / total as f64, highlights as f64 / total as f64)
}

/// Variance of the Laplacian: a standard sharpness proxy.
///
/// Computed over a region so it measures the face rather than a busy
/// background, which would otherwise read as "sharp" on a blurred portrait.
pub fn laplacian_variance(
    rgba: &[u8],
    width: u32,
    height: u32,
    region: Option<(u32, u32, u32, u32)>,
) -> Option<f64> {
    if width < 3 || height < 3 || rgba.len() != width as usize * height as usize * 4 {
        return None;
    }
    let (rx, ry, rw, rh) = region.unwrap_or((0, 0, width, height));
    let x0 = rx.min(width.saturating_sub(1));
    let y0 = ry.min(height.saturating_sub(1));
    let x1 = (x0 + rw).min(width);
    let y1 = (y0 + rh).min(height);
    if x1 <= x0 + 2 || y1 <= y0 + 2 {
        return None;
    }

    let luma = |x: u32, y: u32| -> f64 {
        let i = (y as usize * width as usize + x as usize) * 4;
        0.299 * rgba[i] as f64 + 0.587 * rgba[i + 1] as f64 + 0.114 * rgba[i + 2] as f64
    };

    let mut sum = 0.0;
    let mut sum_sq = 0.0;
    let mut n = 0.0;
    for y in (y0 + 1)..(y1 - 1) {
        for x in (x0 + 1)..(x1 - 1) {
            // 4-neighbour Laplacian.
            let value = luma(x - 1, y) + luma(x + 1, y) + luma(x, y - 1) + luma(x, y + 1)
                - 4.0 * luma(x, y);
            sum += value;
            sum_sq += value * value;
            n += 1.0;
        }
    }
    if n < 1.0 {
        return None;
    }
    let mean = sum / n;
    Some(sum_sq / n - mean * mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for _ in 0..(w * h) {
            v.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        v
    }

    #[test]
    fn neutral_adjustments_change_nothing() {
        let original = solid(8, 8, [90, 120, 200]);
        let mut px = original.clone();
        apply_adjustments(&mut px, &Adjustments::NEUTRAL);
        assert_eq!(px, original);
    }

    #[test]
    fn positive_exposure_brightens() {
        let mut px = solid(4, 4, [100, 100, 100]);
        apply_adjustments(&mut px, &Adjustments { exposure_ev: 1.0, ..Adjustments::NEUTRAL });
        assert!(px[0] > 100, "expected brighter, got {}", px[0]);
    }

    #[test]
    fn negative_exposure_darkens() {
        let mut px = solid(4, 4, [100, 100, 100]);
        apply_adjustments(&mut px, &Adjustments { exposure_ev: -1.0, ..Adjustments::NEUTRAL });
        assert!(px[0] < 100, "expected darker, got {}", px[0]);
    }

    #[test]
    fn alpha_is_never_touched() {
        let mut px = solid(4, 4, [10, 20, 30]);
        apply_adjustments(&mut px, &Adjustments { exposure_ev: 2.0, contrast: 0.5, ..Adjustments::NEUTRAL });
        for chunk in px.chunks_exact(4) {
            assert_eq!(chunk[3], 255);
        }
    }

    #[test]
    fn extreme_values_clamp_instead_of_wrapping() {
        // The classic 8-bit bug: 260 becomes 4 and a highlight turns black.
        let mut px = solid(4, 4, [250, 250, 250]);
        apply_adjustments(&mut px, &Adjustments { exposure_ev: 3.0, ..Adjustments::NEUTRAL });
        for chunk in px.chunks_exact(4) {
            assert_eq!(chunk[0], 255, "value wrapped instead of clamping");
        }

        let mut px = solid(4, 4, [5, 5, 5]);
        apply_adjustments(&mut px, &Adjustments { exposure_ev: -3.0, ..Adjustments::NEUTRAL });
        for chunk in px.chunks_exact(4) {
            assert!(chunk[0] < 5);
        }
    }

    #[test]
    fn nonsense_values_are_ignored() {
        let original = solid(4, 4, [128, 128, 128]);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut px = original.clone();
            apply_adjustments(&mut px, &Adjustments { exposure_ev: bad, ..Adjustments::NEUTRAL });
            assert_eq!(px, original, "{bad} was not rejected");
        }
    }

    #[test]
    fn warming_raises_red_and_lowers_blue() {
        let mut px = solid(4, 4, [128, 128, 128]);
        apply_adjustments(&mut px, &Adjustments { temperature: 0.5, ..Adjustments::NEUTRAL });
        assert!(px[0] > 128, "red should rise when warming");
        assert!(px[2] < 128, "blue should fall when warming");
    }

    #[test]
    fn cooling_does_the_opposite() {
        let mut px = solid(4, 4, [128, 128, 128]);
        apply_adjustments(&mut px, &Adjustments { temperature: -0.5, ..Adjustments::NEUTRAL });
        assert!(px[0] < 128);
        assert!(px[2] > 128);
    }

    #[test]
    fn contrast_pivots_around_mid_grey() {
        // Mid grey must stay put, or raising contrast would also shift
        // brightness.
        let mut px = solid(4, 4, [128, 128, 128]);
        apply_adjustments(&mut px, &Adjustments { contrast: 0.8, ..Adjustments::NEUTRAL });
        assert!((px[0] as i32 - 128).abs() <= 1, "mid grey moved to {}", px[0]);

        // A dark value gets darker, a light one lighter.
        let mut dark = solid(4, 4, [60, 60, 60]);
        apply_adjustments(&mut dark, &Adjustments { contrast: 0.8, ..Adjustments::NEUTRAL });
        assert!(dark[0] < 60);
    }

    #[test]
    fn clipping_reports_blown_highlights() {
        let px = solid(10, 10, [255, 255, 255]);
        let (shadows, highlights) = clipping(&px);
        assert!(highlights > 0.99, "expected all highlights clipped");
        assert_eq!(shadows, 0.0);
    }

    #[test]
    fn clipping_reports_crushed_shadows() {
        let px = solid(10, 10, [0, 0, 0]);
        let (shadows, highlights) = clipping(&px);
        assert!(shadows > 0.99);
        assert_eq!(highlights, 0.0);
    }

    #[test]
    fn a_well_exposed_image_clips_nothing() {
        let px = solid(10, 10, [120, 130, 140]);
        assert_eq!(clipping(&px), (0.0, 0.0));
    }

    #[test]
    fn a_flat_image_has_no_sharpness() {
        let px = solid(16, 16, [128, 128, 128]);
        let v = laplacian_variance(&px, 16, 16, None).unwrap();
        assert!(v < 1.0, "a flat image should not read as sharp: {v}");
    }

    #[test]
    fn a_hard_edge_reads_as_sharp() {
        let mut px = vec![0u8; 16 * 16 * 4];
        for y in 0..16usize {
            for x in 0..16usize {
                let i = (y * 16 + x) * 4;
                let v = if x < 8 { 0 } else { 255 };
                px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let sharp = laplacian_variance(&px, 16, 16, None).unwrap();
        let flat = laplacian_variance(&solid(16, 16, [128, 128, 128]), 16, 16, None).unwrap();
        assert!(sharp > flat * 10.0, "edge {sharp} should far exceed flat {flat}");
    }

    #[test]
    fn sharpness_can_be_limited_to_a_region() {
        // A busy background must not make a blurred face read as sharp.
        let mut px = vec![0u8; 32 * 32 * 4];
        for y in 0..32usize {
            for x in 0..32usize {
                let i = (y * 32 + x) * 4;
                // Left half flat, right half a checkerboard.
                let v = if x < 16 { 128 } else if (x + y) % 2 == 0 { 0 } else { 255 };
                px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let flat_side = laplacian_variance(&px, 32, 32, Some((0, 0, 15, 32))).unwrap();
        let busy_side = laplacian_variance(&px, 32, 32, Some((17, 0, 15, 32))).unwrap();
        assert!(busy_side > flat_side * 10.0);
    }

    #[test]
    fn a_malformed_buffer_is_rejected() {
        assert!(laplacian_variance(&[0, 0, 0, 0], 100, 100, None).is_none());
        assert!(laplacian_variance(&solid(2, 2, [0, 0, 0]), 2, 2, None).is_none());
    }
}
