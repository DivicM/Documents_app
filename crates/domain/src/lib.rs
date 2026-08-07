//! Domain layer: pure geometry and layout logic.
//!
//! Everything in this crate works in millimetres. Pixels and DPI belong to the
//! rendering layer; keeping them out here is what makes the solver testable
//! without an image, a printer, or a UI.

pub mod layout;
pub mod units;

pub use layout::{solve, LayoutConfig, LayoutError, Orientation, Placement, Sheet, SizeMm};
pub use units::{max_lossless_dpi, mm_to_px, mm_to_px_exact, px_to_mm, would_upscale};
