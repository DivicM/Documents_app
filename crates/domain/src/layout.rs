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
    /// Keep the photo exactly as given instead of trying both orientations.
    ///
    /// The solver normally swaps width and height when that fits more copies,
    /// which means asking for 45x35 and asking for 35x45 produce the same
    /// sheet. Setting this makes the requested shape the one that prints, at
    /// the cost of fitting fewer photos.
    #[serde(default)]
    pub lock_orientation: bool,
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

    let grid = if cfg.lock_orientation {
        // The caller asked for this exact shape, so a denser alternative is not
        // an improvement — it would print a different photo size.
        portrait
    } else {
        let landscape =
            build_grid(usable, cfg.photo.swapped(), cfg.gutter_mm, Orientation::Landscape);
        // Prefer portrait on a tie so results stay deterministic and unsurprising.
        if landscape.capacity() > portrait.capacity() { landscape } else { portrait }
    };

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

/// Lay `n` photos out in balanced rows within the grid.
///
/// Filling each row to the grid's width before starting the next leaves a
/// ragged last row: six photos in a four-column grid come out 4 + 2, with a
/// visible gap. Spreading the same six over the same number of rows gives
/// 3 + 3, which reads as deliberate and cuts more predictably.
///
/// Rows are centred individually under [`Alignment::Center`], so an uneven
/// split such as 3 + 2 still looks composed rather than left-heavy.
///
/// Under [`Alignment::Center`] the leftover space is also shared out evenly
/// rather than pushed to the edges: with `k` photos across, the sheet is
/// divided into `k + 1` equal gaps, so the distance between neighbours and the
/// distance to the paper edge are the same. `gutter_mm` acts as a floor, never
/// a fixed value, so photos cannot end up closer together than asked.
///
/// [`Alignment::TopLeft`] keeps the tight packing it exists for: it is there to
/// leave reusable paper, which spreading would defeat.
fn place(cfg: &LayoutConfig, grid: &Grid, usable: SizeMm, n: u32) -> Vec<Placement> {
    if n == 0 || grid.cols == 0 {
        return Vec::new();
    }

    // The number of rows the naive layout would need; keeping it is what makes
    // this a rebalance rather than a change of capacity.
    let rows = n.div_ceil(grid.cols).max(1);

    // Spread as evenly as the count allows: the first `remainder` rows take one
    // extra photo, the rest take the base amount.
    let base = n / rows;
    let remainder = n % rows;

    /// Gap between `count` items and before the first, sharing the slack out
    /// equally over the whole sheet.
    ///
    /// Returns `None` when an equal share would be tighter than `min_gap`,
    /// which means there is nothing to spread and the caller should fall back
    /// to centring a tightly packed block inside the margins. Spreading anyway
    /// would push the block past the paper edge, since the grid was sized
    /// against the usable area but the gaps are measured against the full
    /// sheet.
    fn even_gap(paper: f64, item: f64, count: u32, min_gap: f64) -> Option<f64> {
        if count == 0 {
            return None;
        }
        let slack = paper - count as f64 * item;
        // count + 1 gaps: one before each item and one after the last.
        let gap = slack / (count as f64 + 1.0);
        if gap.is_finite() && gap >= min_gap { Some(gap) } else { None }
    }

    let spread = matches!(cfg.alignment, Alignment::Center);

    // Spread over the whole sheet, not the usable area: the edge gap is one of
    // the gaps being equalised, so subtracting the margin first would count it
    // twice and leave the outer gaps wider than the inner ones. The margin
    // stays a floor, and `even_gap` declines when it cannot be honoured.
    let spread_y = spread
        .then(|| {
            even_gap(cfg.paper.height, grid.photo.height, rows, cfg.gutter_mm.max(cfg.margin_mm))
        })
        .flatten();

    let gap_y = spread_y.unwrap_or(cfg.gutter_mm);
    let block_height = rows as f64 * grid.photo.height + (rows as f64 - 1.0) * gap_y;
    let origin_y = if spread_y.is_some() {
        (cfg.paper.height - block_height) / 2.0
    } else if spread {
        // Nothing to spread: centre the packed block inside the margins.
        cfg.margin_mm + (usable.height - block_height) / 2.0
    } else {
        cfg.margin_mm
    };

    let mut placements = Vec::with_capacity(n as usize);
    for row in 0..rows {
        let in_row = base + if row < remainder { 1 } else { 0 };
        if in_row == 0 {
            continue;
        }

        let spread_x = spread
            .then(|| {
                even_gap(
                    cfg.paper.width,
                    grid.photo.width,
                    in_row,
                    cfg.gutter_mm.max(cfg.margin_mm),
                )
            })
            .flatten();

        let gap_x = spread_x.unwrap_or(cfg.gutter_mm);
        let row_width = in_row as f64 * grid.photo.width + (in_row as f64 - 1.0) * gap_x;
        let origin_x = if spread_x.is_some() {
            (cfg.paper.width - row_width) / 2.0
        } else if spread {
            cfg.margin_mm + (usable.width - row_width) / 2.0
        } else {
            cfg.margin_mm
        };

        for col in 0..in_row {
            placements.push(Placement {
                x_mm: origin_x + col as f64 * (grid.photo.width + gap_x),
                y_mm: origin_y + row as f64 * (grid.photo.height + gap_y),
                size: grid.photo,
                orientation: grid.orientation,
            });
        }
    }

    placements
}

/// One photo size and how many copies of it are wanted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhotoGroup {
    pub size: SizeMm,
    pub count: u32,
}

/// A placement that remembers which group it came from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GroupedPlacement {
    pub placement: Placement,
    /// Index into the `groups` slice passed to [`solve_mixed`].
    pub group: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedSheet {
    pub placements: Vec<GroupedPlacement>,
    /// Copies of each group that did not fit on this sheet.
    pub unplaced: Vec<u32>,
}

/// Choose an orientation for one photo size on a shelf-packed sheet.
///
/// Mirrors what [`solve`] does for the single-size case: try the photo both ways
/// and keep whichever packs more copies. Without this, a mixed sheet holding one
/// size would fit fewer photos than the plain solver on identical input, which
/// would look like the mixed mode losing paper.
fn better_orientation(usable: SizeMm, size: SizeMm, gutter_mm: f64) -> (SizeMm, Orientation) {
    let capacity = |s: SizeMm| {
        fit_count(usable.width, s.width, gutter_mm) * fit_count(usable.height, s.height, gutter_mm)
    };
    // Portrait wins ties, so results stay deterministic and unsurprising.
    if capacity(size.swapped()) > capacity(size) {
        (size.swapped(), Orientation::Landscape)
    } else {
        (size, Orientation::Portrait)
    }
}

/// Arrange several different photo sizes on one sheet (§7).
///
/// Packs in shelves: a row is opened at the current height, photos are placed
/// left to right until the width runs out, then the row closes at the height of
/// its tallest member and the next begins below. Simple and predictable, which
/// matters more here than optimal density — a user can always reorder or use a
/// second sheet.
///
/// Groups are placed largest first, since a big photo squeezed in after the
/// small ones tends to find nowhere to go. Each group is rotated independently
/// if that fits more of it.
pub fn solve_mixed(
    paper: SizeMm,
    groups: &[PhotoGroup],
    margin_mm: f64,
    gutter_mm: f64,
) -> Result<MixedSheet, LayoutError> {
    solve_mixed_with(paper, groups, margin_mm, gutter_mm, false)
}

/// As [`solve_mixed`], but able to keep every group in the orientation given.
///
/// Locking costs capacity; it exists so a deliberately turned frame is not
/// rotated back to whichever way round packs denser.
pub fn solve_mixed_with(
    paper: SizeMm,
    groups: &[PhotoGroup],
    margin_mm: f64,
    gutter_mm: f64,
    lock_orientation: bool,
) -> Result<MixedSheet, LayoutError> {
    if !paper.width.is_finite() || !paper.height.is_finite() || paper.width <= 0.0 || paper.height <= 0.0
    {
        return Err(LayoutError::InvalidDimensions);
    }
    if !margin_mm.is_finite() || margin_mm < 0.0 || !gutter_mm.is_finite() || gutter_mm < 0.0 {
        return Err(LayoutError::InvalidDimensions);
    }
    for g in groups {
        if !g.size.width.is_finite()
            || !g.size.height.is_finite()
            || g.size.width <= 0.0
            || g.size.height <= 0.0
        {
            return Err(LayoutError::InvalidDimensions);
        }
    }

    let usable = SizeMm::new(paper.width - 2.0 * margin_mm, paper.height - 2.0 * margin_mm);
    if usable.width <= 0.0 || usable.height <= 0.0 {
        return Err(LayoutError::NoUsableArea);
    }

    // Largest area first; ties broken by index so the result is deterministic.
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| {
        let area = |i: usize| groups[i].size.width * groups[i].size.height;
        area(b).total_cmp(&area(a)).then(a.cmp(&b))
    });

    let mut placements = Vec::new();
    let mut remaining: Vec<u32> = groups.iter().map(|g| g.count).collect();

    let mut shelf_y = margin_mm;
    let mut cursor_x = margin_mm;
    let mut shelf_height = 0.0f64;

    for &gi in &order {
        let (size, orientation) = if lock_orientation {
            (groups[gi].size, Orientation::Portrait)
        } else {
            better_orientation(usable, groups[gi].size, gutter_mm)
        };
        while remaining[gi] > 0 {
            // Does it fit in the current shelf?
            let needs_gutter = cursor_x > margin_mm;
            let x = if needs_gutter { cursor_x + gutter_mm } else { cursor_x };

            if x + size.width > margin_mm + usable.width + 1e-9 {
                // Close this shelf and open the next.
                if shelf_height <= 0.0 {
                    break; // nothing fitted at all; the photo is too wide
                }
                shelf_y += shelf_height + gutter_mm;
                cursor_x = margin_mm;
                shelf_height = 0.0;
                continue;
            }

            if shelf_y + size.height > margin_mm + usable.height + 1e-9 {
                break; // no vertical room left for this size
            }

            placements.push(GroupedPlacement {
                placement: Placement { x_mm: x, y_mm: shelf_y, size, orientation },
                group: gi,
            });
            cursor_x = x + size.width;
            shelf_height = shelf_height.max(size.height);
            remaining[gi] -= 1;
        }
    }

    Ok(MixedSheet { placements, unplaced: remaining })
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
            lock_orientation: false,
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

        // The gutter is a floor, not a fixed value: a centred sheet shares the
        // leftover space out evenly, so the actual gap is at least the gutter
        // and usually more.
        let gap = sheet.placements[1].x_mm - (sheet.placements[0].x_mm + 35.0);
        assert!(gap >= 5.0 - 1e-9, "gap {gap} is tighter than the gutter");
    }

    fn group(w: f64, h: f64, count: u32) -> PhotoGroup {
        PhotoGroup { size: SizeMm::new(w, h), count }
    }

    /// No placement may escape the margins.
    fn assert_inside(sheet: &MixedSheet, paper: SizeMm, margin: f64) {
        for gp in &sheet.placements {
            let p = gp.placement;
            assert!(p.x_mm >= margin - 1e-9, "left escaped: {}", p.x_mm);
            assert!(p.y_mm >= margin - 1e-9, "top escaped: {}", p.y_mm);
            assert!(
                p.x_mm + p.size.width <= paper.width - margin + 1e-9,
                "right escaped: {}",
                p.x_mm + p.size.width
            );
            assert!(
                p.y_mm + p.size.height <= paper.height - margin + 1e-9,
                "bottom escaped: {}",
                p.y_mm + p.size.height
            );
        }
    }

    /// No two placements may overlap.
    fn assert_no_overlap(sheet: &MixedSheet) {
        let ps: Vec<Placement> = sheet.placements.iter().map(|g| g.placement).collect();
        for i in 0..ps.len() {
            for j in (i + 1)..ps.len() {
                let (a, b) = (ps[i], ps[j]);
                let overlap = a.x_mm + 1e-9 < b.x_mm + b.size.width
                    && b.x_mm + 1e-9 < a.x_mm + a.size.width
                    && a.y_mm + 1e-9 < b.y_mm + b.size.height
                    && b.y_mm + 1e-9 < a.y_mm + a.size.height;
                assert!(!overlap, "placements {i} and {j} overlap: {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn a_mixed_sheet_places_both_sizes() {
        // The brief's example: 4 ID photos plus 2 passport photos together.
        let paper = SizeMm::new(100.0, 150.0);
        let groups = [group(35.0, 45.0, 2), group(30.0, 35.0, 4)];
        let sheet = solve_mixed(paper, &groups, 3.0, 2.0).unwrap();

        assert_eq!(sheet.unplaced, vec![0, 0], "everything should fit");
        assert_eq!(sheet.placements.len(), 6);
        assert_inside(&sheet, paper, 3.0);
        assert_no_overlap(&sheet);
    }

    #[test]
    fn mixed_placements_remember_their_group() {
        let paper = SizeMm::new(100.0, 150.0);
        let groups = [group(35.0, 45.0, 2), group(30.0, 35.0, 3)];
        let sheet = solve_mixed(paper, &groups, 3.0, 2.0).unwrap();

        for gp in &sheet.placements {
            // A rotated group carries its dimensions swapped, so compare against
            // whichever orientation the placement actually used.
            let expected = groups[gp.group].size;
            let matches = match gp.placement.orientation {
                Orientation::Portrait => gp.placement.size == expected,
                Orientation::Landscape => gp.placement.size == expected.swapped(),
            };
            assert!(matches, "placement lost its group size: {gp:?}");
        }
        assert_eq!(sheet.placements.iter().filter(|g| g.group == 0).count(), 2);
        assert_eq!(sheet.placements.iter().filter(|g| g.group == 1).count(), 3);
    }

    /// Swapping the requested size alone is NOT enough to turn the frame: the
    /// solver tries both orientations and picks the denser one, so 35x45 and
    /// 45x35 both come out as 45x35 on 10x15 paper. This documents that, and
    /// is why `lock_orientation` exists.
    #[test]
    fn free_orientation_ignores_which_way_round_the_request_was() {
        let upright = solve(&cfg((100.0, 150.0), (35.0, 45.0), 1)).unwrap();
        let turned = solve(&cfg((100.0, 150.0), (45.0, 35.0), 1)).unwrap();
        assert_eq!(
            upright.placements[0].size, turned.placements[0].size,
            "the solver is supposed to normalise orientation when free to"
        );
    }

    /// With the orientation locked, the requested shape is what prints, even
    /// though the other way round would fit more copies.
    #[test]
    fn locked_orientation_prints_the_requested_shape() {
        let mut c = cfg((100.0, 150.0), (45.0, 35.0), 1);
        c.lock_orientation = true;
        let sheet = solve(&c).unwrap();

        let size = sheet.placements[0].size;
        assert_eq!(size.width, 45.0, "locked frame lost its width");
        assert_eq!(size.height, 35.0, "locked frame lost its height");
        assert_eq!(sheet.orientation, Orientation::Portrait, "locked layout must not rotate");

        // And the same size unlocked would have been turned instead.
        let free = solve(&cfg((100.0, 150.0), (35.0, 45.0), 1)).unwrap();
        assert_eq!(free.placements[0].size, SizeMm::new(45.0, 35.0));
    }

    #[test]
    fn locking_can_cost_capacity() {
        // The trade the setting makes, stated explicitly: 35x45 upright fits 6
        // where the solver would otherwise turn it and fit 8.
        let mut c = cfg((100.0, 150.0), (35.0, 45.0), 20);
        c.lock_orientation = true;
        let locked = solve(&c).unwrap();
        let free = solve(&cfg((100.0, 150.0), (35.0, 45.0), 20)).unwrap();

        assert_eq!(locked.capacity_per_sheet, 6);
        assert_eq!(free.capacity_per_sheet, 8);
    }

    #[test]
    fn mixed_packing_rotates_when_that_fits_more() {
        // The same case as rotating_beats_the_naive_unrotated_grid: 35x45 on
        // 10x15 fits 6 upright but 8 turned. Mixed packing must not be worse
        // than the plain solver on identical input.
        let paper = SizeMm::new(100.0, 150.0);
        let sheet = solve_mixed(paper, &[group(35.0, 45.0, 8)], 0.0, 0.0).unwrap();

        assert_eq!(sheet.placements.len(), 8, "rotation was not tried");
        assert_eq!(sheet.unplaced, vec![0]);
        assert!(sheet
            .placements
            .iter()
            .all(|p| p.placement.orientation == Orientation::Landscape));
        assert_inside(&sheet, paper, 0.0);
        assert_no_overlap(&sheet);
    }

    #[test]
    fn mixed_rotation_never_places_fewer_than_the_plain_solver() {
        // Guards the general property rather than one worked example.
        let paper = SizeMm::new(100.0, 150.0);
        for (w, h) in [(35.0, 45.0), (30.0, 35.0), (45.0, 35.0), (20.0, 60.0)] {
            let plain = solve(&cfg((100.0, 150.0), (w, h), 100)).unwrap();
            let mixed = solve_mixed(paper, &[group(w, h, 100)], 0.0, 0.0).unwrap();
            assert!(
                mixed.placements.len() as u32 >= plain.capacity_per_sheet,
                "{w}x{h}: mixed fitted {} but the plain solver fits {}",
                mixed.placements.len(),
                plain.capacity_per_sheet
            );
        }
    }

    #[test]
    fn what_does_not_fit_is_reported_not_dropped() {
        // Silently printing fewer copies than asked would be the worst outcome.
        let paper = SizeMm::new(100.0, 150.0);
        let groups = [group(35.0, 45.0, 50)];
        let sheet = solve_mixed(paper, &groups, 3.0, 2.0).unwrap();

        let placed = sheet.placements.len() as u32;
        assert!(placed > 0 && placed < 50);
        assert_eq!(placed + sheet.unplaced[0], 50, "copies went missing");
    }

    #[test]
    fn a_single_group_still_fills_the_sheet() {
        // Mixed packing must not be worse than the plain solver for one size.
        let paper = SizeMm::new(100.0, 150.0);
        let sheet = solve_mixed(paper, &[group(35.0, 45.0, 6)], 0.0, 0.0).unwrap();
        assert_eq!(sheet.placements.len(), 6, "6 of 35x45 should fit on 10x15");
        assert_inside(&sheet, paper, 0.0);
        assert_no_overlap(&sheet);
    }

    #[test]
    fn a_photo_larger_than_the_paper_is_left_unplaced() {
        let paper = SizeMm::new(50.0, 50.0);
        let sheet = solve_mixed(paper, &[group(80.0, 80.0, 3)], 0.0, 0.0).unwrap();
        assert!(sheet.placements.is_empty());
        assert_eq!(sheet.unplaced, vec![3]);
    }

    #[test]
    fn mixed_rejects_nonsense_dimensions() {
        let paper = SizeMm::new(100.0, 150.0);
        assert_eq!(
            solve_mixed(paper, &[group(f64::NAN, 45.0, 1)], 0.0, 0.0),
            Err(LayoutError::InvalidDimensions)
        );
        assert_eq!(
            solve_mixed(paper, &[group(35.0, 45.0, 1)], 80.0, 0.0),
            Err(LayoutError::NoUsableArea)
        );
    }

    #[test]
    fn empty_groups_produce_an_empty_sheet() {
        let sheet = solve_mixed(SizeMm::new(100.0, 150.0), &[], 3.0, 2.0).unwrap();
        assert!(sheet.placements.is_empty());
        assert!(sheet.unplaced.is_empty());
    }

    #[test]
    fn gutters_are_respected_between_mixed_photos() {
        let paper = SizeMm::new(100.0, 150.0);
        let sheet = solve_mixed(paper, &[group(20.0, 20.0, 2)], 0.0, 5.0).unwrap();
        assert_eq!(sheet.placements.len(), 2);
        let a = sheet.placements[0].placement;
        let b = sheet.placements[1].placement;
        let gap = b.x_mm - (a.x_mm + a.size.width);
        assert!((gap - 5.0).abs() < 1e-9, "gutter was {gap}, expected 5");
    }

    /// Count the photos on each row of a solved sheet, top to bottom.
    fn row_counts(sheet: &Sheet) -> Vec<usize> {
        let mut rows: Vec<(f64, usize)> = Vec::new();
        for p in &sheet.placements {
            match rows.iter_mut().find(|(y, _)| (*y - p.y_mm).abs() < 1e-6) {
                Some((_, n)) => *n += 1,
                None => rows.push((p.y_mm, 1)),
            }
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|(_, n)| n).collect()
    }

    /// The Citizen CY-02 case: its paper with the app's default margins, which
    /// is the layout the gap was reported on.
    fn citizen(count: u32) -> LayoutConfig {
        let mut c = cfg((156.1, 105.0), (35.0, 45.0), count);
        c.margin_mm = 3.0;
        c.gutter_mm = 2.0;
        c
    }

    #[test]
    fn a_partial_sheet_balances_its_rows() {
        // The reported problem: six photos in a four-column grid came out 4 + 2
        // with a conspicuous gap. The same six should read as 3 + 3.
        let sheet = solve(&citizen(6)).unwrap();
        assert_eq!(sheet.capacity_per_sheet, 8, "expected a 4x2 grid on this paper");
        assert_eq!(row_counts(&sheet), vec![3, 3]);
    }

    #[test]
    fn an_odd_count_splits_as_evenly_as_it_can() {
        // Five over two rows cannot be equal; 3 + 2 is the closest, and the
        // fuller row must come first so the gap is at the bottom.
        let sheet = solve(&citizen(5)).unwrap();
        assert_eq!(row_counts(&sheet), vec![3, 2]);
    }

    #[test]
    fn a_full_sheet_is_unchanged_by_balancing() {
        // Rebalancing must not disturb the common case of a full sheet.
        let sheet = solve(&citizen(8)).unwrap();
        assert_eq!(row_counts(&sheet), vec![4, 4]);
    }

    #[test]
    fn balanced_rows_stay_inside_the_margins() {
        // Centring each row separately must not push any of them off the sheet.
        let c = citizen(5);
        let sheet = solve(&c).unwrap();

        for p in &sheet.placements {
            assert!(p.x_mm >= c.margin_mm - 1e-9, "left escaped: {}", p.x_mm);
            assert!(p.y_mm >= c.margin_mm - 1e-9, "top escaped: {}", p.y_mm);
            assert!(
                p.x_mm + p.size.width <= c.paper.width - c.margin_mm + 1e-9,
                "right escaped: {}",
                p.x_mm + p.size.width
            );
            assert!(
                p.y_mm + p.size.height <= c.paper.height - c.margin_mm + 1e-9,
                "bottom escaped: {}",
                p.y_mm + p.size.height
            );
        }
    }

    #[test]
    fn balancing_never_loses_or_invents_a_photo() {
        // The rebalance rewrites the loop that emits placements, so the count
        // it produces is worth pinning across a range of inputs.
        for n in 1..=8u32 {
            let sheet = solve(&citizen(n)).unwrap();
            assert_eq!(sheet.placements.len(), n as usize, "wrong count for {n}");
            assert_eq!(row_counts(&sheet).iter().sum::<usize>(), n as usize);
        }
    }

    #[test]
    fn spacing_is_even_across_a_row_including_the_edges() {
        // The request: every photo the same distance from its neighbours and
        // from the paper edge, rather than a tight block with the slack pushed
        // to the sides.
        let sheet = solve(&citizen(6)).unwrap();
        let row: Vec<Placement> = {
            let y = sheet.placements[0].y_mm;
            let mut r: Vec<Placement> = sheet
                .placements
                .iter()
                .copied()
                .filter(|p| (p.y_mm - y).abs() < 1e-6)
                .collect();
            r.sort_by(|a, b| a.x_mm.total_cmp(&b.x_mm));
            r
        };
        assert!(row.len() >= 2, "need a multi-photo row to measure gaps");

        let paper_w = 156.1;
        let mut gaps = vec![row[0].x_mm];
        for pair in row.windows(2) {
            gaps.push(pair[1].x_mm - (pair[0].x_mm + pair[0].size.width));
        }
        let last = row.last().unwrap();
        gaps.push(paper_w - (last.x_mm + last.size.width));

        let first = gaps[0];
        for (i, g) in gaps.iter().enumerate() {
            assert!(
                (g - first).abs() < 1e-6,
                "gap {i} is {g}, expected {first}; gaps: {gaps:?}"
            );
        }
    }

    #[test]
    fn spacing_is_even_down_the_sheet_including_the_edges() {
        let sheet = solve(&citizen(6)).unwrap();
        let mut ys: Vec<f64> = Vec::new();
        for p in &sheet.placements {
            if !ys.iter().any(|y| (y - p.y_mm).abs() < 1e-6) {
                ys.push(p.y_mm);
            }
        }
        ys.sort_by(|a, b| a.total_cmp(b));
        assert!(ys.len() >= 2, "need at least two rows");

        let paper_h = 105.0;
        let photo_h = sheet.placements[0].size.height;
        let mut gaps = vec![ys[0]];
        for pair in ys.windows(2) {
            gaps.push(pair[1] - (pair[0] + photo_h));
        }
        gaps.push(paper_h - (ys.last().unwrap() + photo_h));

        let first = gaps[0];
        for (i, g) in gaps.iter().enumerate() {
            assert!((g - first).abs() < 1e-6, "row gap {i} is {g}, expected {first}");
        }
    }

    #[test]
    fn even_spacing_never_goes_below_the_requested_gutter() {
        // On a full sheet there is no slack to share, so the gutter must hold
        // rather than being computed down to something tighter.
        let c = citizen(8);
        let sheet = solve(&c).unwrap();
        let mut row: Vec<Placement> = sheet
            .placements
            .iter()
            .copied()
            .filter(|p| (p.y_mm - sheet.placements[0].y_mm).abs() < 1e-6)
            .collect();
        row.sort_by(|a, b| a.x_mm.total_cmp(&b.x_mm));

        for pair in row.windows(2) {
            let gap = pair[1].x_mm - (pair[0].x_mm + pair[0].size.width);
            assert!(gap >= c.gutter_mm - 1e-9, "gap {gap} is tighter than the gutter");
        }
    }

    #[test]
    fn topleft_alignment_still_packs_tightly() {
        // Spreading is for centred sheets. TopLeft exists to leave reusable
        // paper, which spreading would defeat.
        let mut c = citizen(4);
        c.alignment = Alignment::TopLeft;
        let sheet = solve(&c).unwrap();

        let mut row: Vec<Placement> = sheet
            .placements
            .iter()
            .copied()
            .filter(|p| (p.y_mm - sheet.placements[0].y_mm).abs() < 1e-6)
            .collect();
        row.sort_by(|a, b| a.x_mm.total_cmp(&b.x_mm));

        assert!((row[0].x_mm - c.margin_mm).abs() < 1e-9, "did not start at the margin");
        for pair in row.windows(2) {
            let gap = pair[1].x_mm - (pair[0].x_mm + pair[0].size.width);
            assert!((gap - c.gutter_mm).abs() < 1e-9, "gap {gap} is not the gutter");
        }
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
