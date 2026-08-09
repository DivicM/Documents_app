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
    use ort::ep::DirectML;

    let builder = Session::builder().ok()?;
    // Registration succeeds even when no suitable device exists, so the real
    // check is whether the model commits.
    let mut builder = builder
        .with_execution_providers([DirectML::default().build()])
        .ok()?;
    builder.commit_from_file(path).ok()
}

#[cfg(not(windows))]
fn try_directml(_path: &std::path::Path) -> Option<Session> {
    None
}

/// Fixed input size of this export. Read from the model, not chosen.
pub const INPUT_SIZE: u32 = 1024;

/// ImageNet normalisation, which BiRefNet was trained with.
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

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Produce a mask at the model's own resolution.
    ///
    /// Not upscaled to the source size here: the caller knows what resolution
    /// it needs, and a mask stretched too early would waste memory on a
    /// print-sized image.
    pub fn segment(&mut self, image: &RgbImage) -> Result<AlphaMask, SegmentError> {
        let input = preprocess(image);

        let tensor = Value::from_array((
            [1_usize, 3, INPUT_SIZE as usize, INPUT_SIZE as usize],
            input,
        ))
        .map_err(|e| SegmentError::Inference(e.to_string()))?;

        let outputs = self
            .session
            .run(ort::inputs!["input_image" => tensor])
            .map_err(|e| SegmentError::Inference(e.to_string()))?;

        let (_, data) = outputs
            .get("output_image")
            .ok_or_else(|| SegmentError::UnexpectedOutput("missing output_image".into()))?
            .try_extract_tensor::<f32>()
            .map_err(|e| SegmentError::UnexpectedOutput(e.to_string()))?;

        let expected = (INPUT_SIZE * INPUT_SIZE) as usize;
        if data.len() < expected {
            return Err(SegmentError::UnexpectedOutput(format!(
                "expected {expected} values, got {}",
                data.len()
            )));
        }

        // The model emits logits; a sigmoid turns them into coverage.
        let bytes: Vec<u8> = data[..expected]
            .iter()
            .map(|&v| {
                let p = 1.0 / (1.0 + (-v).exp());
                (p * 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect();

        AlphaMask::new(INPUT_SIZE, INPUT_SIZE, bytes)
            .ok_or_else(|| SegmentError::UnexpectedOutput("mask size mismatch".into()))
    }
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
