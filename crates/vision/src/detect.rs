//! Running YuNet through ONNX Runtime.
//!
//! Thin by design: preprocessing and decoding live in `decode`, which is
//! testable without a model. This module only moves tensors.

use crate::decode::{
    combined_score, decode_yunet, letterbox, DecodeParams, Letterbox, RawAnchor, STRIDES,
};
use domain::geometry::Detection;
use image::RgbImage;
use ort::session::Session;
use ort::value::Value;

/// Model input size, fixed at 640x640 by this ONNX export.
///
/// Read from the model rather than chosen: the graph rejects any other shape.
/// A larger input also detects smaller faces, which matters for photos taken
/// at a distance.
pub const INPUT_WIDTH: u32 = 640;
pub const INPUT_HEIGHT: u32 = 640;

#[derive(Debug)]
pub enum DetectError {
    ModelLoad(String),
    Inference(String),
    /// An expected output tensor was missing or the wrong shape.
    UnexpectedOutput(String),
}

impl core::fmt::Display for DetectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ModelLoad(m) => write!(f, "could not load model: {m}"),
            Self::Inference(m) => write!(f, "inference failed: {m}"),
            Self::UnexpectedOutput(m) => write!(f, "unexpected model output: {m}"),
        }
    }
}

impl std::error::Error for DetectError {}

pub struct FaceDetector {
    session: Session,
    params: DecodeParams,
}

impl FaceDetector {
    /// Load the model from a bundled file.
    pub fn from_path(path: &std::path::Path) -> Result<Self, DetectError> {
        let session = Session::builder()
            .and_then(|mut b| b.commit_from_file(path))
            .map_err(|e| DetectError::ModelLoad(e.to_string()))?;
        Ok(Self { session, params: DecodeParams::default() })
    }

    pub fn with_params(mut self, params: DecodeParams) -> Self {
        self.params = params;
        self
    }

    /// Detect faces, returning boxes and landmarks in source-image pixels.
    pub fn detect(&mut self, image: &RgbImage) -> Result<Vec<Detection>, DetectError> {
        let lb = letterbox(image.width(), image.height(), INPUT_WIDTH, INPUT_HEIGHT);
        let input = preprocess(image, &lb);

        let tensor = Value::from_array(([1_usize, 3, INPUT_HEIGHT as usize, INPUT_WIDTH as usize], input))
            .map_err(|e| DetectError::Inference(e.to_string()))?;

        let outputs = self
            .session
            .run(ort::inputs!["input" => tensor])
            .map_err(|e| DetectError::Inference(e.to_string()))?;

        let mut anchors = Vec::new();
        for stride in STRIDES {
            collect_anchors(&outputs, stride, &mut anchors)?;
        }

        Ok(decode_yunet(&anchors, &lb, &self.params))
    }
}

/// Resize into the letterbox and lay out as CHW float, which is what the model
/// expects. Padding is mid-grey so it does not read as a strong edge.
fn preprocess(image: &RgbImage, lb: &Letterbox) -> Vec<f32> {
    let w = INPUT_WIDTH as usize;
    let h = INPUT_HEIGHT as usize;
    let mut out = vec![128.0f32; 3 * w * h];

    let scaled_w = (image.width() as f64 * lb.scale).round() as u32;
    let scaled_h = (image.height() as f64 * lb.scale).round() as u32;
    if scaled_w == 0 || scaled_h == 0 {
        return out;
    }

    let resized = image::imageops::resize(
        image,
        scaled_w,
        scaled_h,
        image::imageops::FilterType::Triangle,
    );

    for y in 0..scaled_h {
        let dst_y = y as usize + lb.pad_y as usize;
        if dst_y >= h {
            break;
        }
        for x in 0..scaled_w {
            let dst_x = x as usize + lb.pad_x as usize;
            if dst_x >= w {
                break;
            }
            let px = resized.get_pixel(x, y);
            // YuNet takes raw 0-255 BGR values, not normalised RGB.
            out[dst_y * w + dst_x] = px[2] as f32;
            out[w * h + dst_y * w + dst_x] = px[1] as f32;
            out[2 * w * h + dst_y * w + dst_x] = px[0] as f32;
        }
    }
    out
}

/// Pull one stride's four tensors and turn them into anchors.
fn collect_anchors(
    outputs: &ort::session::SessionOutputs,
    stride: u32,
    into: &mut Vec<RawAnchor>,
) -> Result<(), DetectError> {
    let get = |name: &str| -> Result<&[f32], DetectError> {
        outputs
            .get(name)
            .ok_or_else(|| DetectError::UnexpectedOutput(format!("missing output {name}")))?
            .try_extract_tensor::<f32>()
            .map(|(_, data)| data)
            .map_err(|e| DetectError::UnexpectedOutput(format!("{name}: {e}")))
    };

    let cls = get(&format!("cls_{stride}"))?;
    let obj = get(&format!("obj_{stride}"))?;
    let bbox = get(&format!("bbox_{stride}"))?;
    let kps = get(&format!("kps_{stride}"))?;

    let cols = INPUT_WIDTH.div_ceil(stride);
    let rows = INPUT_HEIGHT.div_ceil(stride);
    let expected = (cols * rows) as usize;

    if cls.len() < expected || obj.len() < expected {
        return Err(DetectError::UnexpectedOutput(format!(
            "stride {stride}: expected {expected} anchors, got cls {} obj {}",
            cls.len(),
            obj.len()
        )));
    }
    if bbox.len() < expected * 4 || kps.len() < expected * 10 {
        return Err(DetectError::UnexpectedOutput(format!(
            "stride {stride}: bbox {} kps {} too short for {expected} anchors",
            bbox.len(),
            kps.len()
        )));
    }

    for idx in 0..expected {
        let score = combined_score(cls[idx], obj[idx]);
        into.push(RawAnchor {
            score,
            bbox: [bbox[idx * 4], bbox[idx * 4 + 1], bbox[idx * 4 + 2], bbox[idx * 4 + 3]],
            landmarks: {
                let mut lm = [0.0f32; 10];
                lm.copy_from_slice(&kps[idx * 10..idx * 10 + 10]);
                lm
            },
            stride,
            col: (idx as u32) % cols,
            row: (idx as u32) / cols,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_produces_the_expected_buffer_size() {
        let img = RgbImage::new(640, 480);
        let lb = letterbox(640, 480, INPUT_WIDTH, INPUT_HEIGHT);
        let buf = preprocess(&img, &lb);
        assert_eq!(buf.len(), 3 * INPUT_WIDTH as usize * INPUT_HEIGHT as usize);
    }

    #[test]
    fn padding_region_stays_neutral_grey() {
        // A wide image leaves horizontal bands top and bottom; those must not
        // read as black, which the detector could mistake for structure.
        let img = RgbImage::from_pixel(640, 240, image::Rgb([255, 255, 255]));
        let lb = letterbox(640, 240, INPUT_WIDTH, INPUT_HEIGHT);
        let buf = preprocess(&img, &lb);
        // Top-left corner is padding for this aspect ratio.
        assert_eq!(buf[0], 128.0);
    }

    #[test]
    fn channels_are_written_in_bgr_order() {
        // A pure red image must land in the third channel plane, not the first.
        let img = RgbImage::from_pixel(INPUT_WIDTH, INPUT_HEIGHT, image::Rgb([255, 0, 0]));
        let lb = letterbox(INPUT_WIDTH, INPUT_HEIGHT, INPUT_WIDTH, INPUT_HEIGHT);
        let buf = preprocess(&img, &lb);
        let plane = (INPUT_WIDTH * INPUT_HEIGHT) as usize;
        let centre = (INPUT_HEIGHT / 2) as usize * INPUT_WIDTH as usize + (INPUT_WIDTH / 2) as usize;
        assert_eq!(buf[centre], 0.0, "blue plane should be empty for red input");
        assert_eq!(buf[2 * plane + centre], 255.0, "red belongs in the last plane");
    }
}
