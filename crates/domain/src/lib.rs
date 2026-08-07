//! Domain layer: pure geometry and layout logic.
//!
//! Everything in this crate works in millimetres. Pixels and DPI belong to the
//! rendering layer; keeping them out here is what makes the solver testable
//! without an image, a printer, or a UI.

pub mod geometry;
pub mod layout;
pub mod overrides;
pub mod render;
pub mod resample;
pub mod units;

pub use geometry::{
    crop_overflow, estimate_head_anchors, solve_crop, CropError, CropTarget, Detection,
    FaceLandmarks, HeadAnchors, Point, Rect,
};
pub use layout::{solve, LayoutConfig, LayoutError, Orientation, Placement, Sheet, SizeMm};
pub use overrides::{effective, AutoFlags, FaceState, Overrides};
pub use render::{
    placement_to_pixels, render_sheet, render_sheet_with_photo, CalibrationScale, PhotoSource,
    PixelRect, PrintableOrigin, Raster, RenderParams,
};
pub use resample::{resample_region, ImageBuf, ImageRef};
pub use units::{max_lossless_dpi, mm_to_px, mm_to_px_exact, px_to_mm, would_upscale};
