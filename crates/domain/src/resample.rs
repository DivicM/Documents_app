//! Resampling a cropped region to its printed size.
//!
//! Lanczos3 because this is the print path: bilinear softens detail that a
//! 600dpi printer would otherwise resolve. Separable, so a two-pass horizontal
//! then vertical filter costs far less than a true 2D convolution and gives the
//! same result.

/// A borrowed RGBA image.
#[derive(Debug, Clone, Copy)]
pub struct ImageRef<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
}

impl<'a> ImageRef<'a> {
    pub fn new(pixels: &'a [u8], width: u32, height: u32) -> Option<Self> {
        if pixels.len() != width as usize * height as usize * 4 {
            return None;
        }
        Some(Self { pixels, width, height })
    }

    fn sample(&self, x: u32, y: u32, channel: usize) -> f32 {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let idx = (y as usize * self.width as usize + x as usize) * 4 + channel;
        self.pixels[idx] as f32
    }
}

/// An owned RGBA image.
#[derive(Debug, Clone)]
pub struct ImageBuf {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

const LANCZOS_A: f32 = 3.0;

fn sinc(x: f32) -> f32 {
    if x.abs() < 1e-8 {
        1.0
    } else {
        let px = core::f32::consts::PI * x;
        px.sin() / px
    }
}

/// Lanczos kernel with a = 3.
fn lanczos(x: f32) -> f32 {
    let x = x.abs();
    if x >= LANCZOS_A {
        0.0
    } else {
        sinc(x) * sinc(x / LANCZOS_A)
    }
}

/// Weights for one output position along one axis.
struct Contribution {
    start: i64,
    weights: Vec<f32>,
}

/// Precompute filter weights for a whole axis.
///
/// Done once per axis rather than per pixel: every output column shares the
/// same weights as every other in its row.
fn compute_contributions(src_len: u32, dst_len: u32, src_start: f64, src_extent: f64) -> Vec<Contribution> {
    let scale = src_extent / dst_len as f64;
    // When downscaling, the filter must widen to average the pixels being
    // discarded, otherwise the result aliases.
    let filter_scale = scale.max(1.0);
    let support = LANCZOS_A as f64 * filter_scale;

    (0..dst_len)
        .map(|i| {
            // Centre of this output pixel, in source coordinates.
            let centre = src_start + (i as f64 + 0.5) * scale;
            let left = ((centre - support) + 0.5).floor() as i64;
            let right = ((centre + support) - 0.5).ceil() as i64;

            let mut weights = Vec::with_capacity((right - left + 1).max(0) as usize);
            let mut total = 0.0f32;
            for s in left..=right {
                let dist = (s as f64 + 0.5 - centre) / filter_scale;
                let w = lanczos(dist as f32);
                weights.push(w);
                total += w;
            }
            // Normalise so flat areas keep their brightness.
            if total.abs() > 1e-8 {
                for w in &mut weights {
                    *w /= total;
                }
            }
            let _ = src_len;
            Contribution { start: left, weights }
        })
        .collect()
}

/// Resample a rectangular region of `src` into a `dst_w` x `dst_h` image.
///
/// The region is given in source pixels and may be fractional, which matters
/// because a crop computed in millimetres rarely lands on whole pixels.
pub fn resample_region(
    src: &ImageRef<'_>,
    region_x: f64,
    region_y: f64,
    region_w: f64,
    region_h: f64,
    dst_w: u32,
    dst_h: u32,
) -> ImageBuf {
    if dst_w == 0 || dst_h == 0 || src.width == 0 || src.height == 0 {
        return ImageBuf { pixels: Vec::new(), width: 0, height: 0 };
    }

    let x_contrib = compute_contributions(src.width, dst_w, region_x, region_w);
    let y_contrib = compute_contributions(src.height, dst_h, region_y, region_h);

    // Horizontal pass into an intermediate the height of the source region.
    let band_top = (region_y.floor() as i64 - LANCZOS_A as i64 - 1).max(i64::MIN / 2);
    let band_bottom = ((region_y + region_h).ceil() as i64 + LANCZOS_A as i64 + 1).min(i64::MAX / 2);
    let band_height = (band_bottom - band_top).max(1) as usize;

    let mut horizontal = vec![0.0f32; dst_w as usize * band_height * 4];
    for (row, y_abs) in (band_top..band_bottom).enumerate() {
        let sy = y_abs.clamp(0, src.height as i64 - 1) as u32;
        for (col, c) in x_contrib.iter().enumerate() {
            let mut acc = [0.0f32; 4];
            for (k, w) in c.weights.iter().enumerate() {
                let sx = (c.start + k as i64).clamp(0, src.width as i64 - 1) as u32;
                for (ch, a) in acc.iter_mut().enumerate() {
                    *a += src.sample(sx, sy, ch) * w;
                }
            }
            let base = (row * dst_w as usize + col) * 4;
            horizontal[base..base + 4].copy_from_slice(&acc);
        }
    }

    // Vertical pass into the final image.
    let mut out = vec![0u8; dst_w as usize * dst_h as usize * 4];
    for (row, c) in y_contrib.iter().enumerate() {
        for col in 0..dst_w as usize {
            let mut acc = [0.0f32; 4];
            for (k, w) in c.weights.iter().enumerate() {
                let y_abs = c.start + k as i64;
                let band_row = (y_abs - band_top).clamp(0, band_height as i64 - 1) as usize;
                let base = (band_row * dst_w as usize + col) * 4;
                for ch in 0..4 {
                    acc[ch] += horizontal[base + ch] * w;
                }
            }
            let base = (row * dst_w as usize + col) * 4;
            for ch in 0..4 {
                // Lanczos overshoots at sharp edges; clamping is what keeps
                // that from wrapping around into black or white speckle.
                out[base + ch] = acc[ch].round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    ImageBuf { pixels: out, width: dst_w, height: dst_h }
}

/// Resample a rotated region.
///
/// The region is described by its centre, size and angle in source pixels, so
/// a crop can be straightened without a separate rotate-then-crop pass that
/// would resample twice and soften the result.
///
/// Bilinear rather than Lanczos here: the rotated sampling grid makes a
/// separable filter inapplicable, and a true rotated Lanczos convolution costs
/// far more than the visible difference at print resolution.
#[allow(clippy::too_many_arguments)]
pub fn resample_rotated(
    src: &ImageRef<'_>,
    centre_x: f64,
    centre_y: f64,
    region_w: f64,
    region_h: f64,
    angle_deg: f64,
    dst_w: u32,
    dst_h: u32,
) -> ImageBuf {
    if dst_w == 0 || dst_h == 0 || src.width == 0 || src.height == 0 {
        return ImageBuf { pixels: Vec::new(), width: 0, height: 0 };
    }

    // Rotating the crop by -angle undoes a head tilted by +angle.
    let theta = -angle_deg.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();
    let sx_step = region_w / dst_w as f64;
    let sy_step = region_h / dst_h as f64;

    let mut out = vec![0u8; dst_w as usize * dst_h as usize * 4];
    for row in 0..dst_h {
        for col in 0..dst_w {
            // Offset from the crop centre, in source units.
            let dx = (col as f64 + 0.5) * sx_step - region_w / 2.0;
            let dy = (row as f64 + 0.5) * sy_step - region_h / 2.0;
            let sx = centre_x + dx * cos_t - dy * sin_t;
            let sy = centre_y + dx * sin_t + dy * cos_t;

            let px = bilinear(src, sx, sy);
            let base = (row as usize * dst_w as usize + col as usize) * 4;
            out[base..base + 4].copy_from_slice(&px);
        }
    }

    ImageBuf { pixels: out, width: dst_w, height: dst_h }
}

/// Bilinear sample with edge clamping.
fn bilinear(src: &ImageRef<'_>, x: f64, y: f64) -> [u8; 4] {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = (x - x0) as f32;
    let fy = (y - y0) as f32;

    let clamp_x = |v: f64| v.clamp(0.0, (src.width - 1) as f64) as u32;
    let clamp_y = |v: f64| v.clamp(0.0, (src.height - 1) as f64) as u32;
    let (x0i, x1i) = (clamp_x(x0), clamp_x(x0 + 1.0));
    let (y0i, y1i) = (clamp_y(y0), clamp_y(y0 + 1.0));

    let mut out = [0u8; 4];
    for (ch, o) in out.iter_mut().enumerate() {
        let top = src.sample(x0i, y0i, ch) * (1.0 - fx) + src.sample(x1i, y0i, ch) * fx;
        let bottom = src.sample(x0i, y1i, ch) * (1.0 - fx) + src.sample(x1i, y1i, ch) * fx;
        *o = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, colour: [u8; 4]) -> Vec<u8> {
        colour.iter().cycle().take(w as usize * h as usize * 4).copied().collect()
    }

    #[test]
    fn flat_colour_survives_resampling() {
        // The classic filter bug: weights that do not sum to 1 darken or
        // brighten flat areas.
        let px = solid(64, 64, [120, 130, 140, 255]);
        let src = ImageRef::new(&px, 64, 64).unwrap();
        let out = resample_region(&src, 0.0, 0.0, 64.0, 64.0, 32, 32);

        for chunk in out.pixels.chunks_exact(4) {
            assert!((chunk[0] as i32 - 120).abs() <= 1, "red drifted: {}", chunk[0]);
            assert!((chunk[1] as i32 - 130).abs() <= 1, "green drifted: {}", chunk[1]);
            assert!((chunk[2] as i32 - 140).abs() <= 1, "blue drifted: {}", chunk[2]);
            assert_eq!(chunk[3], 255, "alpha changed");
        }
    }

    #[test]
    fn output_has_the_requested_dimensions() {
        let px = solid(100, 80, [255, 0, 0, 255]);
        let src = ImageRef::new(&px, 100, 80).unwrap();
        let out = resample_region(&src, 0.0, 0.0, 100.0, 80.0, 413, 531);
        assert_eq!((out.width, out.height), (413, 531));
        assert_eq!(out.pixels.len(), 413 * 531 * 4);
    }

    #[test]
    fn a_region_selects_the_right_part_of_the_image() {
        // Left half red, right half blue. Cropping the right half must give
        // blue only.
        let mut px = vec![0u8; 40 * 10 * 4];
        for y in 0..10 {
            for x in 0..40 {
                let i = (y * 40 + x) * 4;
                let c: [u8; 4] = if x < 20 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                px[i..i + 4].copy_from_slice(&c);
            }
        }
        let src = ImageRef::new(&px, 40, 10).unwrap();
        // Well inside the right half, clear of the boundary the filter blurs.
        let out = resample_region(&src, 25.0, 0.0, 10.0, 10.0, 8, 8);

        let centre = ((4 * 8) + 4) * 4;
        assert!(out.pixels[centre + 2] > 200, "expected blue, got {:?}", &out.pixels[centre..centre + 4]);
        assert!(out.pixels[centre] < 55, "red leaked in");
    }

    #[test]
    fn upscaling_preserves_a_flat_colour() {
        let px = solid(8, 8, [200, 100, 50, 255]);
        let src = ImageRef::new(&px, 8, 8).unwrap();
        let out = resample_region(&src, 0.0, 0.0, 8.0, 8.0, 64, 64);
        let centre = ((32 * 64) + 32) * 4;
        assert!((out.pixels[centre] as i32 - 200).abs() <= 2);
        assert!((out.pixels[centre + 1] as i32 - 100).abs() <= 2);
    }

    #[test]
    fn heavy_downscale_does_not_alias_to_one_extreme() {
        // A fine checkerboard downscaled hard should average towards grey, not
        // collapse to pure black or white as nearest-neighbour would.
        let mut px = vec![0u8; 128 * 128 * 4];
        for y in 0..128 {
            for x in 0..128 {
                let i = (y * 128 + x) * 4;
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let src = ImageRef::new(&px, 128, 128).unwrap();
        let out = resample_region(&src, 0.0, 0.0, 128.0, 128.0, 16, 16);

        let centre = ((8 * 16) + 8) * 4;
        let v = out.pixels[centre];
        assert!((60..=195).contains(&v), "aliased to {v} instead of averaging");
    }

    #[test]
    fn fractional_regions_are_accepted() {
        // Crops computed in millimetres almost never land on whole pixels.
        let px = solid(50, 50, [10, 20, 30, 255]);
        let src = ImageRef::new(&px, 50, 50).unwrap();
        let out = resample_region(&src, 10.7, 3.2, 20.4, 18.9, 32, 32);
        assert_eq!((out.width, out.height), (32, 32));
        let centre = ((16 * 32) + 16) * 4;
        assert!((out.pixels[centre] as i32 - 10).abs() <= 1);
    }

    #[test]
    fn a_region_reaching_past_the_edge_is_clamped_not_wrapped() {
        // Sampling outside the image must repeat the edge, never wrap round to
        // the opposite side, which would put a stripe of the wrong content in.
        let mut px = vec![0u8; 20 * 20 * 4];
        for y in 0..20 {
            for x in 0..20 {
                let i = (y * 20 + x) * 4;
                let c: [u8; 4] = if x < 3 { [255, 255, 255, 255] } else { [0, 0, 0, 255] };
                px[i..i + 4].copy_from_slice(&c);
            }
        }
        let src = ImageRef::new(&px, 20, 20).unwrap();
        // Region hangs off the right edge, where the image is black.
        let out = resample_region(&src, 15.0, 5.0, 10.0, 10.0, 8, 8);
        let right_edge = ((4 * 8) + 7) * 4;
        assert!(
            out.pixels[right_edge] < 60,
            "edge clamp wrapped to the white left side: {}",
            out.pixels[right_edge]
        );
    }

    #[test]
    fn zero_rotation_matches_the_unrotated_path() {
        // A no-op rotation must not shift or blur the image, or straightening
        // by 0 degrees would still degrade it.
        let px = solid(40, 40, [90, 120, 200, 255]);
        let src = ImageRef::new(&px, 40, 40).unwrap();
        let out = resample_rotated(&src, 20.0, 20.0, 20.0, 20.0, 0.0, 20, 20);

        let centre = ((10 * 20) + 10) * 4;
        assert_eq!(out.pixels[centre], 90);
        assert_eq!(out.pixels[centre + 1], 120);
        assert_eq!(out.pixels[centre + 2], 200);
    }

    #[test]
    fn rotation_straightens_a_tilted_edge() {
        // A diagonal boundary rotated by its own angle should become vertical:
        // the left column ends up one colour and the right column the other.
        let mut px = vec![0u8; 200 * 200 * 4];
        for y in 0..200usize {
            for x in 0..200usize {
                let i = (y * 200 + x) * 4;
                // Boundary along y = x, i.e. tilted 45 degrees.
                let c: [u8; 4] =
                    if (x as f64) < (y as f64) { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                px[i..i + 4].copy_from_slice(&c);
            }
        }
        let src = ImageRef::new(&px, 200, 200).unwrap();
        // The boundary runs at +45 degrees, so straightening passes +45.
        let out = resample_rotated(&src, 100.0, 100.0, 80.0, 80.0, 45.0, 40, 40);

        // After straightening, sample well left and right of centre.
        let left = ((20 * 40) + 6) * 4;
        let right = ((20 * 40) + 33) * 4;
        assert!(
            out.pixels[left] > 200 && out.pixels[left + 2] < 60,
            "left should be red: {:?}",
            &out.pixels[left..left + 4]
        );
        assert!(
            out.pixels[right + 2] > 200 && out.pixels[right] < 60,
            "right should be blue: {:?}",
            &out.pixels[right..right + 4]
        );
    }

    #[test]
    fn rotation_keeps_a_flat_colour_flat() {
        let px = solid(60, 60, [77, 88, 99, 255]);
        let src = ImageRef::new(&px, 60, 60).unwrap();
        let out = resample_rotated(&src, 30.0, 30.0, 30.0, 30.0, 12.5, 30, 30);
        for chunk in out.pixels.chunks_exact(4) {
            assert!((chunk[0] as i32 - 77).abs() <= 1, "drifted to {}", chunk[0]);
        }
    }

    #[test]
    fn rotation_samples_outside_the_image_are_clamped() {
        // Rotating near a corner pulls the sampling grid off the image; that
        // must repeat the edge rather than read out of bounds.
        let px = solid(30, 30, [200, 10, 10, 255]);
        let src = ImageRef::new(&px, 30, 30).unwrap();
        let out = resample_rotated(&src, 1.0, 1.0, 20.0, 20.0, 30.0, 16, 16);
        assert_eq!(out.pixels.len(), 16 * 16 * 4);
        // Everything is one colour, so clamped samples must match it.
        let corner = 0;
        assert!((out.pixels[corner] as i32 - 200).abs() <= 1);
    }

    #[test]
    fn mismatched_buffer_is_rejected() {
        let px = vec![0u8; 10];
        assert!(ImageRef::new(&px, 100, 100).is_none());
    }

    #[test]
    fn zero_sized_output_is_handled() {
        let px = solid(10, 10, [1, 2, 3, 255]);
        let src = ImageRef::new(&px, 10, 10).unwrap();
        let out = resample_region(&src, 0.0, 0.0, 10.0, 10.0, 0, 5);
        assert_eq!(out.pixels.len(), 0);
    }
}
