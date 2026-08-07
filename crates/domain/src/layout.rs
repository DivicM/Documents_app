//! Layout solver: arranges N copies of a photo onto a sheet of paper.
//!
//! All dimensions are millimetres. The solver is deliberately ignorant of DPI,
//! pixels, printers and images so that it can be tested in isolation.

use serde::{Deserialize, Serialize};

/// Rectangular size in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SizeMm {
    pub width: f64,
    pub height: f64,
}

impl SizeMm {
    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    fn swapped(self) -> Self {
        Self { width: self.height, height: self.width }
    }
}

/// Which way a photo is rotated on the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    /// Photo placed as authored.
    Portrait,
    /// Photo rotated 90 degrees.
    Landscape,
}

/// How leftover space is distributed when the sheet is not full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alignment {
    /// Centre the block of photos on the sheet.
    Center,
    /// Push the block into the top-left corner, leaving reusable paper.
    TopLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub paper: SizeMm,
    pub photo: SizeMm,
    pub count: u32,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    pub alignment: Alignment,
}

/// One photo positioned on the sheet. Origin is the top-left of the paper.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub x_mm: f64,
    pub y_mm: f64,
    pub size: SizeMm,
    pub orientation: Orientation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub placements: Vec<Placement>,
    pub orientation: Orientation,
    /// How many photos fit on one sheet in the chosen orientation.
    pub capacity_per_sheet: u32,
    /// Sheets needed for the requested count.
    pub sheets_needed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// A single photo does not fit the usable area in either orientation.
    PhotoTooLarge,
    /// Config contained a non-finite or non-positive value.
    InvalidDimensions,
    /// Margins consume the whole sheet.
    NoUsableArea,
}

/// Rows and columns that fit, for one candidate orientation.
struct Grid {
    cols: u32,
    rows: u32,
    photo: SizeMm,
    orientation: Orientation,
}

impl Grid {
    fn capacity(&self) -> u32 {
        self.cols * self.rows
    }
}

/// How many items of `item` fit across `available` with `gutter` between them.
///
/// Solves `n*item + (n-1)*gutter <= available` for the largest integer n.
fn fit_count(available: f64, item: f64, gutter: f64) -> u32 {
    if item <= 0.0 || available < item {
        return 0;
    }
    // Adding one gutter to both sides turns the uneven series into a clean
    // division: n*(item+gutter) <= available+gutter.
    let n = (available + gutter) / (item + gutter);
    // Guard against a value like 2.9999999996 collapsing to 2.
    let floored = n.floor();
    let candidate = if (n - floored).abs() < 1e-9 || (floored + 1.0 - n).abs() < 1e-9 {
        n.round()
    } else {
        floored
    };
    // Re-verify: floating point must never hand back a grid that does not fit.
    let candidate = candidate.max(0.0) as u32;
    if candidate == 0 {
        return 0;
    }
    let used = candidate as f64 * item + (candidate as f64 - 1.0) * gutter;
    if used <= available + 1e-9 {
        candidate
    } else {
        candidate - 1
    }
}

fn build_grid(usable: SizeMm, photo: SizeMm, gutter: f64, orientation: Orientation) -> Grid {
    Grid {
        cols: fit_count(usable.width, photo.width, gutter),
        rows: fit_count(usable.height, photo.height, gutter),
        photo,
        orientation,
    }
}

/// Arrange `count` copies of a photo onto paper.
///
/// Tries both orientations and keeps the one with the greater capacity. When a
/// single sheet cannot hold `count`, the layout still succeeds and reports
/// `sheets_needed` so the caller can decide what to do — printing silently
/// truncated output would be worse than surfacing the number.
pub fn solve(cfg: &LayoutConfig) -> Result<Sheet, LayoutError> {
    validate(cfg)?;

    let usable = SizeMm::new(
        cfg.paper.width - 2.0 * cfg.margin_mm,
        cfg.paper.height - 2.0 * cfg.margin_mm,
    );
    if usable.width <= 0.0 || usable.height <= 0.0 {
        return Err(LayoutError::NoUsableArea);
    }

    let portrait = build_grid(usable, cfg.photo, cfg.gutter_mm, Orientation::Portrait);
    let landscape = build_grid(usable, cfg.photo.swapped(), cfg.gutter_mm, Orientation::Landscape);

    // Prefer portrait on a tie so results stay deterministic and unsurprising.
    let grid = if landscape.capacity() > portrait.capacity() { landscape } else { portrait };

    if grid.capacity() == 0 {
        return Err(LayoutError::PhotoTooLarge);
    }

    let capacity = grid.capacity();
    let on_first_sheet = cfg.count.min(capacity);
    let sheets_needed = cfg.count.div_ceil(capacity);

    Ok(Sheet {
        placements: place(cfg, &grid, usable, on_first_sheet),
        orientation: grid.orientation,
        capacity_per_sheet: capacity,
        sheets_needed,
    })
}

/// Lay `n` photos out row-major within the grid.
fn place(cfg: &LayoutConfig, grid: &Grid, usable: SizeMm, n: u32) -> Vec<Placement> {
    if n == 0 {
        return Vec::new();
    }

    // Only the rows and columns actually occupied should be centred, otherwise
    // a half-empty sheet would centre around empty space.
    let used_cols = n.min(grid.cols);
    let used_rows = n.div_ceil(grid.cols);

    let block_width =
        used_cols as f64 * grid.photo.width + (used_cols as f64 - 1.0) * cfg.gutter_mm;
    let block_height =
        used_rows as f64 * grid.photo.height + (used_rows as f64 - 1.0) * cfg.gutter_mm;

    let (origin_x, origin_y) = match cfg.alignment {
        Alignment::TopLeft => (cfg.margin_mm, cfg.margin_mm),
        Alignment::Center => (
            cfg.margin_mm + (usable.width - block_width) / 2.0,
            cfg.margin_mm + (usable.height - block_height) / 2.0,
        ),
    };

    (0..n)
        .map(|i| {
            let col = i % grid.cols;
            let row = i / grid.cols;
            Placement {
                x_mm: origin_x + col as f64 * (grid.photo.width + cfg.gutter_mm),
                y_mm: origin_y + row as f64 * (grid.photo.height + cfg.gutter_mm),
                size: grid.photo,
                orientation: grid.orientation,
            }
        })
        .collect()
}

fn validate(cfg: &LayoutConfig) -> Result<(), LayoutError> {
    let dims = [
        cfg.paper.width,
        cfg.paper.height,
        cfg.photo.width,
        cfg.photo.height,
    ];
    if dims.iter().any(|d| !d.is_finite() || *d <= 0.0) {
        return Err(LayoutError::InvalidDimensions);
    }
    if !cfg.margin_mm.is_finite()
        || cfg.margin_mm < 0.0
        || !cfg.gutter_mm.is_finite()
        || cfg.gutter_mm < 0.0
    {
        return Err(LayoutError::InvalidDimensions);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(paper: (f64, f64), photo: (f64, f64), count: u32) -> LayoutConfig {
        LayoutConfig {
            paper: SizeMm::new(paper.0, paper.1),
            photo: SizeMm::new(photo.0, photo.1),
            count,
            margin_mm: 0.0,
            gutter_mm: 0.0,
            alignment: Alignment::Center,
        }
    }

    #[test]
    fn rotating_beats_the_naive_unrotated_grid() {
        // 10x15 paper with a 35x45 photo. Unrotated this is 2x3=6, which is the
        // figure quoted in the brief, but rotating to 45x35 gives 2x4=8 and
        // still fits (90mm of 100mm across, 140mm of 150mm down). The solver is
        // required to try both orientations, so 8 is the correct answer.
        let sheet = solve(&cfg((100.0, 150.0), (35.0, 45.0), 8)).unwrap();
        assert_eq!(sheet.capacity_per_sheet, 8);
        assert_eq!(sheet.orientation, Orientation::Landscape);
        assert_eq!(sheet.placements.len(), 8);
        assert_eq!(sheet.sheets_needed, 1);
    }

    #[test]
    fn unrotated_grid_alone_would_fit_six() {
        // Pins the brief's arithmetic: with rotation forbidden (square-ish paper
        // that gains nothing from swapping), the 2x3 reasoning still holds.
        let sheet = solve(&cfg((100.0, 135.0), (35.0, 45.0), 6)).unwrap();
        assert_eq!(sheet.capacity_per_sheet, 6);
        assert_eq!(sheet.orientation, Orientation::Portrait);
    }

    #[test]
    fn picks_orientation_with_greater_capacity() {
        // 100x150 with a 45x35 photo: portrait gives 2x4=8, landscape 2x3=6.
        let sheet = solve(&cfg((100.0, 150.0), (45.0, 35.0), 8)).unwrap();
        assert_eq!(sheet.orientation, Orientation::Portrait);
        assert_eq!(sheet.capacity_per_sheet, 8);
    }

    #[test]
    fn exact_fit_is_not_lost_to_float_error() {
        // 3 * 33.3333 is 99.9999, and must still yield 3 across a 100mm sheet.
        let sheet = solve(&cfg((100.0, 100.0), (100.0 / 3.0, 100.0), 3)).unwrap();
        assert_eq!(sheet.capacity_per_sheet, 3);
    }

    #[test]
    fn count_beyond_capacity_reports_extra_sheets() {
        // Capacity is 8 per sheet (see rotating_beats_the_naive_unrotated_grid).
        let sheet = solve(&cfg((100.0, 150.0), (35.0, 45.0), 17)).unwrap();
        assert_eq!(sheet.capacity_per_sheet, 8);
        assert_eq!(sheet.sheets_needed, 3);
        // Only the first sheet is laid out.
        assert_eq!(sheet.placements.len(), 8);
    }

    #[test]
    fn photo_larger_than_paper_is_an_error() {
        assert_eq!(
            solve(&cfg((50.0, 50.0), (60.0, 60.0), 1)),
            Err(LayoutError::PhotoTooLarge)
        );
    }

    #[test]
    fn margins_consuming_sheet_is_an_error() {
        let mut c = cfg((100.0, 150.0), (35.0, 45.0), 1);
        c.margin_mm = 60.0;
        assert_eq!(solve(&c), Err(LayoutError::NoUsableArea));
    }

    #[test]
    fn zero_count_yields_no_placements() {
        let sheet = solve(&cfg((100.0, 150.0), (35.0, 45.0), 0)).unwrap();
        assert!(sheet.placements.is_empty());
        assert_eq!(sheet.sheets_needed, 0);
    }

    #[test]
    fn nonfinite_dimensions_rejected() {
        assert_eq!(
            solve(&cfg((f64::NAN, 150.0), (35.0, 45.0), 1)),
            Err(LayoutError::InvalidDimensions)
        );
    }

    #[test]
    fn gutters_reduce_capacity() {
        let mut c = cfg((100.0, 150.0), (35.0, 45.0), 6);
        c.gutter_mm = 5.0;
        let sheet = solve(&c).unwrap();
        // 2*35+5=75 fits across; 3*45+10=145 fits down. Still 6.
        assert_eq!(sheet.capacity_per_sheet, 6);
        let gap = sheet.placements[1].x_mm - (sheet.placements[0].x_mm + 35.0);
        assert!((gap - 5.0).abs() < 1e-9, "gutter not applied: {gap}");
    }

    #[test]
    fn topleft_alignment_starts_at_margin() {
        let mut c = cfg((100.0, 150.0), (35.0, 45.0), 1);
        c.margin_mm = 4.0;
        c.alignment = Alignment::TopLeft;
        let sheet = solve(&c).unwrap();
        assert!((sheet.placements[0].x_mm - 4.0).abs() < 1e-9);
        assert!((sheet.placements[0].y_mm - 4.0).abs() < 1e-9);
    }
}
