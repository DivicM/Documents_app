//! Face detection. Everything model-related lives here so the layers above it
//! stay testable without ONNX Runtime or a model file.

pub mod decode;
pub mod detect;
pub mod segment;

pub use decode::{
    combined_score, decode_yunet, letterbox, primary_face, DecodeParams, Letterbox, RawAnchor,
};
pub use detect::{DetectError, FaceDetector};
pub use segment::{BackgroundSegmenter, SegmentError};
