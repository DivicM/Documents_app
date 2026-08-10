//! Property-based tests for the layout solver.
//!
//! These assert invariants that must hold for every valid input, not just the
//! cases someone thought to write down: nothing escapes the margins, nothing
//! overlaps, and the reported capacity matches the placements produced.

use domain::layout::{solve, Alignment, LayoutConfig, Orientation, Placement, SizeMm};
use proptest::prelude::*;

const EPS: f64 = 1e-6;

fn config_strategy() -> impl Strategy<Value = LayoutConfig> {
    (
        10.0f64..500.0, // paper width
        10.0f64..500.0, // paper height
        5.0f64..200.0,  // photo width
        5.0f64..200.0,  // photo height
        0u32..40,       // count
        0.0f64..15.0,   // margin
        0.0f64..15.0,   // gutter
        prop::bool::ANY,
        // Locked orientation must satisfy the same invariants as free.
        prop::bool::ANY,
    )
        .prop_map(|(pw, ph, iw, ih, count, margin, gutter, center, locked)| LayoutConfig {
            paper: SizeMm::new(pw, ph),
            photo: SizeMm::new(iw, ih),
            count,
            margin_mm: margin,
            gutter_mm: gutter,
            alignment: if center { Alignment::Center } else { Alignment::TopLeft },
            lock_orientation: locked,
        })
}

fn overlaps(a: &Placement, b: &Placement) -> bool {
    a.x_mm + EPS < b.x_mm + b.size.width
        && b.x_mm + EPS < a.x_mm + a.size.width
        && a.y_mm + EPS < b.y_mm + b.size.height
        && b.y_mm + EPS < a.y_mm + a.size.height
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// No photo may extend past the margin on any side.
    #[test]
    fn placements_stay_inside_margins(cfg in config_strategy()) {
        if let Ok(sheet) = solve(&cfg) {
            for p in &sheet.placements {
                prop_assert!(p.x_mm >= cfg.margin_mm - EPS, "left edge escaped: {}", p.x_mm);
                prop_assert!(p.y_mm >= cfg.margin_mm - EPS, "top edge escaped: {}", p.y_mm);
                prop_assert!(
                    p.x_mm + p.size.width <= cfg.paper.width - cfg.margin_mm + EPS,
                    "right edge escaped: {} > {}",
                    p.x_mm + p.size.width,
                    cfg.paper.width - cfg.margin_mm
                );
                prop_assert!(
                    p.y_mm + p.size.height <= cfg.paper.height - cfg.margin_mm + EPS,
                    "bottom edge escaped: {} > {}",
                    p.y_mm + p.size.height,
                    cfg.paper.height - cfg.margin_mm
                );
            }
        }
    }

    /// No two photos may overlap.
    #[test]
    fn placements_never_overlap(cfg in config_strategy()) {
        if let Ok(sheet) = solve(&cfg) {
            let ps = &sheet.placements;
            for i in 0..ps.len() {
                for j in (i + 1)..ps.len() {
                    prop_assert!(!overlaps(&ps[i], &ps[j]), "overlap {i}/{j}: {:?} {:?}", ps[i], ps[j]);
                }
            }
        }
    }

    /// Placement count equals min(count, capacity), and sheet count is ceil.
    #[test]
    fn capacity_and_sheet_count_are_consistent(cfg in config_strategy()) {
        if let Ok(sheet) = solve(&cfg) {
            let expected = cfg.count.min(sheet.capacity_per_sheet);
            prop_assert_eq!(sheet.placements.len() as u32, expected);

            let expected_sheets = if cfg.count == 0 {
                0
            } else {
                cfg.count.div_ceil(sheet.capacity_per_sheet)
            };
            prop_assert_eq!(sheet.sheets_needed, expected_sheets);
        }
    }

    /// Every photo carries the sheet's orientation, and its dimensions match
    /// that orientation. This is what stops a rotated layout from being printed
    /// with unrotated pixel data.
    #[test]
    fn placement_sizes_match_orientation(cfg in config_strategy()) {
        if let Ok(sheet) = solve(&cfg) {
            for p in &sheet.placements {
                prop_assert_eq!(p.orientation, sheet.orientation);
                let (w, h) = match sheet.orientation {
                    Orientation::Portrait => (cfg.photo.width, cfg.photo.height),
                    Orientation::Landscape => (cfg.photo.height, cfg.photo.width),
                };
                prop_assert!((p.size.width - w).abs() < EPS);
                prop_assert!((p.size.height - h).abs() < EPS);
            }
        }
    }

    /// The solver must never pick the worse orientation. Recomputing capacity
    /// naively for both and taking the max has to agree with what it returned.
    ///
    /// Only when it is free to choose: `lock_orientation` deliberately gives up
    /// capacity to keep the requested shape, and is covered separately.
    #[test]
    fn chosen_capacity_is_the_maximum_available(cfg in config_strategy()) {
        if cfg.lock_orientation {
            return Ok(());
        }
        if let Ok(sheet) = solve(&cfg) {
            let usable_w = cfg.paper.width - 2.0 * cfg.margin_mm;
            let usable_h = cfg.paper.height - 2.0 * cfg.margin_mm;

            let fits = |avail: f64, item: f64| -> u32 {
                if item <= 0.0 || avail < item { return 0; }
                let mut n = 0u32;
                // Count up rather than divide, to stay independent of the
                // implementation being tested.
                while (n + 1) as f64 * item + n as f64 * cfg.gutter_mm <= avail + EPS {
                    n += 1;
                }
                n
            };

            let portrait = fits(usable_w, cfg.photo.width) * fits(usable_h, cfg.photo.height);
            let landscape = fits(usable_w, cfg.photo.height) * fits(usable_h, cfg.photo.width);
            prop_assert_eq!(sheet.capacity_per_sheet, portrait.max(landscape));
        }
    }

    /// A locked layout prints exactly the shape it was given and never rotates.
    /// This is the whole point of the flag, so it is asserted over the same
    /// generated space as everything else rather than on one example.
    #[test]
    fn locked_layouts_keep_the_requested_shape(cfg in config_strategy()) {
        if !cfg.lock_orientation {
            return Ok(());
        }
        if let Ok(sheet) = solve(&cfg) {
            prop_assert_eq!(sheet.orientation, Orientation::Portrait);
            for p in &sheet.placements {
                prop_assert!((p.size.width - cfg.photo.width).abs() < EPS);
                prop_assert!((p.size.height - cfg.photo.height).abs() < EPS);
            }
        }
    }
}
