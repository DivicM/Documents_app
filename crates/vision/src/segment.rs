//! Background segmentation with BiRefNet.
//!
//! Produces an alpha mask separating subject from background. As with face
//! detection, this module only moves tensors: the mask model itself decides
//! nothing about how the mask is used.

use domain::mask::AlphaMask;
use image::RgbImage;
use ort::session::Session;
use ort::value::Value;

/// Threads to use for inference: all physical cores, leaving one for the UI so
/// the window does not freeze while a mask is computed.
fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get().saturating_sub(1)).max(1))
        .unwrap_or(1)
}

/// Try to build a DirectML session, returning None if the GPU path is not
/// usable on this machine.
#[cfg(windows)]
fn try_directml(path: &std::path::Path) -> Option<Session> {
    use ort::ep::directml::PerformancePreference;
    use ort::ep::DirectML;

    let builder = Session::builder().ok()?;
    // HighPerformance rather than the default: a laptop typically reports its
    // integrated GPU first, and the default preference picks that one. On a
    // machine with a single GPU this changes nothing.
    let mut builder = builder
        .with_execution_providers([DirectML::default()
            .with_performance_preference(PerformancePreference::HighPerformance)
            .build()])
        .ok()?;
    builder.commit_from_file(path).ok()
}

#[cfg(not(windows))]
fn try_directml(_path: &std::path::Path) -> Option<Session> {
    None
}

/// Resolution the segmenter runs at.
///
/// U2NETP is fully convolutional and will accept other sizes, but 320 is what
/// it was trained on and the only size where it is reliable: measured at 640 on
/// a test portrait, the head stayed solid while the torso broke up into a
/// half-transparent, patchy mask. Higher input resolution is not a route to a
/// finer mask here.
///
/// The magnification to print size is handled by sampling the mask bilinearly
/// rather than by enlarging the model input.
///
/// Also well under the two-second Windows TDR limit; exceeding it is what made
/// the earlier BiRefNet export (1024x1024, ~7s) reset the GPU driver mid-run.
pub const INPUT_SIZE: u32 = 320;

/// Input tensor name. U2NETP was exported from PyTorch without naming it.
const INPUT_NAME: &str = "input.1";

/// The fused prediction. U2-Net is deep-supervised and emits seven maps; the
/// first is the fusion of the rest and the only one worth reading.
const OUTPUT_NAME: &str = "1959";

/// ImageNet normalisation, as U-2-Net was trained with.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

#[derive(Debug)]
pub enum SegmentError {
    ModelLoad(String),
    Inference(String),
    UnexpectedOutput(String),
}

impl core::fmt::Display for SegmentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ModelLoad(m) => write!(f, "could not load segmentation model: {m}"),
            Self::Inference(m) => write!(f, "segmentation failed: {m}"),
            Self::UnexpectedOutput(m) => write!(f, "unexpected model output: {m}"),
        }
    }
}

impl std::error::Error for SegmentError {}

/// Which execution provider ended up running the model.
///
/// Surfaced so the UI can explain a slow first run rather than appear stuck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    DirectMl,
    Cpu,
}

pub struct BackgroundSegmenter {
    session: Session,
    backend: Backend,
}

impl BackgroundSegmenter {
    pub fn from_path(path: &std::path::Path) -> Result<Self, SegmentError> {
        Self::with_threads(path, default_threads())
    }

    /// Load the model, preferring the GPU and falling back to the CPU.
    ///
    /// This model is a transformer at 1024x1024, which takes tens of seconds on
    /// a CPU and a couple of seconds on a GPU, so the difference is worth the
    /// extra attempt. Registration failing is not an error: a machine without a
    /// suitable GPU simply runs on the CPU.
    pub fn with_threads(path: &std::path::Path, threads: usize) -> Result<Self, SegmentError> {
        if cfg!(windows) {
            if let Some(session) = try_directml(path) {
                return Ok(Self { session, backend: Backend::DirectMl });
            }
        }

        let builder = Session::builder()
            .map_err(|e| SegmentError::ModelLoad(e.to_string()))?;
        let mut builder = builder
            .with_intra_threads(threads.max(1))
            .map_err(|e| SegmentError::ModelLoad(e.to_string()))?;
        let session = builder
            .commit_from_file(path)
            .map_err(|e| SegmentError::ModelLoad(e.to_string()))?;
        Ok(Self { session, backend: Backend::Cpu })
    }

    /// Load on the CPU provider only, skipping the GPU attempt.
    ///
    /// Exists so the two backends can be timed against each other on the same
    /// machine; the application itself always prefers the GPU.
    #[doc(hidden)]
    pub fn cpu_only(path: &std::path::Path) -> Result<Self, SegmentError> {
        let builder = Session::builder().map_err(|e| SegmentError::ModelLoad(e.to_string()))?;
        let mut builder = builder
            .with_intra_threads(default_threads())
            .map_err(|e| SegmentError::ModelLoad(e.to_string()))?;
        let session = builder
            .commit_from_file(path)
            .map_err(|e| SegmentError::ModelLoad(e.to_string()))?;
        Ok(Self { session, backend: Backend::Cpu })
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// As [`segment`], but reading RGBA pixels directly.
    ///
    /// Skips building an intermediate `RgbImage`, which for a 1067x1600 photo
    /// meant copying about 5MB before doing anything useful. The model resizes
    /// to `INPUT_SIZE` regardless, so that copy only ever fed the resampler.
    pub fn segment_rgba(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<AlphaMask, SegmentError> {
        let input = preprocess_rgba(rgba, width, height);
        self.run(input)
    }

    /// Produce a mask at the model's own resolution.
    ///
    /// Not upscaled to the source size here: the caller knows what resolution
    /// it needs, and a mask stretched too early would waste memory on a
    /// print-sized image.
    pub fn segment(&mut self, image: &RgbImage) -> Result<AlphaMask, SegmentError> {
        self.run(preprocess(image))
    }

    /// Run the model on an already-normalised CHW tensor.
    ///
    /// Shared by both entry points so the two cannot drift apart in how they
    /// read the output.
    fn run(&mut self, input: Vec<f32>) -> Result<AlphaMask, SegmentError> {
        let tensor = Value::from_array((
            [1_usize, 3, INPUT_SIZE as usize, INPUT_SIZE as usize],
            input,
        ))
        .map_err(|e| SegmentError::Inference(e.to_string()))?;

        let outputs = self
            .session
            .run(ort::inputs![INPUT_NAME => tensor])
            .map_err(|e| SegmentError::Inference(e.to_string()))?;

        let (_, data) = outputs
            .get(OUTPUT_NAME)
            .ok_or_else(|| {
                SegmentError::UnexpectedOutput(format!("missing output {OUTPUT_NAME}"))
            })?
            .try_extract_tensor::<f32>()
            .map_err(|e| SegmentError::UnexpectedOutput(e.to_string()))?;

        let expected = (INPUT_SIZE * INPUT_SIZE) as usize;
        if data.len() < expected {
            return Err(SegmentError::UnexpectedOutput(format!(
                "expected {expected} values, got {}",
                data.len()
            )));
        }
        let data = &data[..expected];

        // U-2-Net applies its own sigmoid, so these are already probabilities,
        // but they rarely span the full range. Rescaling to min-max is what the
        // reference implementation does; without it a mask can come out uniformly
        // grey and the threshold slider has nothing to bite on.
        let bytes = normalise_to_bytes(data);

        AlphaMask::new(INPUT_SIZE, INPUT_SIZE, bytes)
            .ok_or_else(|| SegmentError::UnexpectedOutput("mask size mismatch".into()))
    }
}

/// Rescale a prediction map to the full 0-255 byte range.
///
/// U-2-Net applies its own sigmoid, so these are already probabilities, but they
/// rarely span the whole range. Min-max rescaling is what the reference
/// implementation does; without it a mask comes out uniformly grey and the
/// threshold slider has nothing to bite on.
fn normalise_to_bytes(data: &[f32]) -> Vec<u8> {
    let (min, max) = data
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = max - min;

    if span > 1e-6 {
        data.iter()
            .map(|&v| (((v - min) / span) * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect()
    } else {
        // A flat prediction carries no information. Mid-grey is honest about
        // that; stretching it would amplify noise into a convincing fake mask.
        vec![128; data.len()]
    }
}

/// Preprocessing alone, exposed so the timing example can separate it from
/// inference. Not part of the normal path.
#[doc(hidden)]
pub fn preprocess_for_bench(image: &RgbImage) -> Vec<f32> {
    preprocess(image)
}

/// As [`preprocess`], but sampling RGBA pixels directly.
///
/// Uses nearest-neighbour rather than the `image` crate's triangle filter. At
/// this reduction — roughly 1600px down to 320 — the model is unaffected by the
/// difference, and it avoids materialising an intermediate image.
fn preprocess_rgba(rgba: &[u8], width: u32, height: u32) -> Vec<f32> {
    let n = INPUT_SIZE as usize;
    let mut out = vec![0.0f32; 3 * n * n];
    if width == 0 || height == 0 {
        return out;
    }

    let plane = n * n;
    for y in 0..n {
        // Sample from the centre of each destination pixel.
        let sy = (((y as f32 + 0.5) / n as f32) * height as f32) as u32;
        let sy = sy.min(height - 1) as usize;
        for x in 0..n {
            let sx = (((x as f32 + 0.5) / n as f32) * width as f32) as u32;
            let sx = sx.min(width - 1) as usize;
            let i = (sy * width as usize + sx) * 4;
            let o = y * n + x;
            for c in 0..3 {
                let v = rgba[i + c] as f32 / 255.0;
                out[c * plane + o] = (v - MEAN[c]) / STD[c];
            }
        }
    }
    out
}

/// Resize to the model input and normalise, laid out as CHW float.
///
/// Stretches rather than letterboxes: the mask is mapped back over the whole
/// image, so preserving aspect ratio here would leave padding bands that would
/// have to be tracked and removed again.
fn preprocess(image: &RgbImage) -> Vec<f32> {
    let n = INPUT_SIZE as usize;
    let resized = image::imageops::resize(
        image,
        INPUT_SIZE,
        INPUT_SIZE,
        image::imageops::FilterType::Triangle,
    );

    let mut out = vec![0.0f32; 3 * n * n];
    for y in 0..n {
        for x in 0..n {
            let px = resized.get_pixel(x as u32, y as u32);
            for c in 0..3 {
                let v = px[c] as f32 / 255.0;
                out[c * n * n + y * n + x] = (v - MEAN[c]) / STD[c];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_produces_the_expected_buffer_size() {
        let img = RgbImage::new(640, 480);
        let buf = preprocess(&img);
        assert_eq!(buf.len(), 3 * INPUT_SIZE as usize * INPUT_SIZE as usize);
    }

    #[test]
    fn normalisation_centres_mid_grey_near_zero() {
        // A mid-grey image should land close to zero after ImageNet
        // normalisation; a wildly different value means the constants or the
        // divide are wrong.
        let img = RgbImage::from_pixel(64, 64, image::Rgb([124, 116, 104]));
        let buf = preprocess(&img);
        let n = INPUT_SIZE as usize;
        let centre = (n / 2) * n + n / 2;
        for c in 0..3 {
            let v = buf[c * n * n + centre];
            assert!(v.abs() < 0.15, "channel {c} normalised to {v}");
        }
    }

    #[test]
    fn rgba_preprocessing_matches_the_rgb_path() {
        // Two entry points feed the same model. A flat image makes them
        // directly comparable: any difference here would be a bug in the
        // channel order or the normalisation, not in the resampler.
        let rgb = RgbImage::from_pixel(64, 48, image::Rgb([200, 120, 40]));
        let mut rgba = Vec::with_capacity(64 * 48 * 4);
        for _ in 0..(64 * 48) {
            rgba.extend_from_slice(&[200, 120, 40, 255]);
        }

        let from_rgb = preprocess(&rgb);
        let from_rgba = preprocess_rgba(&rgba, 64, 48);

        assert_eq!(from_rgb.len(), from_rgba.len());
        for (i, (a, b)) in from_rgb.iter().zip(from_rgba.iter()).enumerate() {
            assert!((a - b).abs() < 1e-5, "differ at {i}: {a} vs {b}");
        }
    }

    #[test]
    fn rgba_preprocessing_keeps_channels_in_rgb_order() {
        // Alpha must be skipped, not folded into a channel.
        let mut rgba = Vec::new();
        for _ in 0..(8 * 8) {
            rgba.extend_from_slice(&[255, 0, 0, 255]);
        }
        let buf = preprocess_rgba(&rgba, 8, 8);
        let n = INPUT_SIZE as usize;
        let centre = (n / 2) * n + n / 2;
        assert!(buf[centre] > buf[n * n + centre], "red should lead");
    }

    #[test]
    fn normalisation_stretches_a_narrow_range_to_full_scale() {
        // U-2-Net output often sits in a narrow band. Without stretching, the
        // mask is uniformly grey and the threshold slider does nothing.
        let out = normalise_to_bytes(&[0.40, 0.45, 0.50, 0.55, 0.60]);
        assert_eq!(out.first(), Some(&0));
        assert_eq!(out.last(), Some(&255));
    }

    #[test]
    fn a_flat_prediction_becomes_mid_grey_not_noise() {
        // Dividing by a zero span would produce NaN or amplify float dust into
        // a mask that looks meaningful but is not.
        let out = normalise_to_bytes(&[0.5; 16]);
        assert!(out.iter().all(|&v| v == 128), "flat input produced {out:?}");
    }

    #[test]
    fn normalisation_preserves_ordering() {
        // Whatever the scaling, a more confident pixel must stay more confident.
        let out = normalise_to_bytes(&[0.1, 0.9, 0.3, 0.7]);
        assert!(out[0] < out[2] && out[2] < out[3] && out[3] < out[1], "{out:?}");
    }

    #[test]
    fn channels_are_written_in_rgb_order() {
        // Unlike the face detector, this model takes RGB.
        let img = RgbImage::from_pixel(32, 32, image::Rgb([255, 0, 0]));
        let buf = preprocess(&img);
        let n = INPUT_SIZE as usize;
        let centre = (n / 2) * n + n / 2;
        let red = buf[centre];
        let green = buf[n * n + centre];
        assert!(red > green, "red should be the first plane: {red} vs {green}");
    }
}
