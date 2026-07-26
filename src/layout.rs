//! Guillotine layout planner, ported from herdr-nvim: given the pane
//! rectangles of a tab, produce an anchor pane plus the ordered split steps
//! that rebuild the same layout from that anchor. Used to squeeze the whole
//! existing layout into the left portion of the tab so the gitview sidebar
//! can take the right portion at a fixed ratio.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneRect {
    pub pane_id: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Right,
    Down,
}

impl Dir {
    pub fn as_cli_arg(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

/// One `pane move` needed to rebuild the layout: split `target` in `dir`,
/// `target` keeping `ratio` of the region, placing `pane` in the rest.
#[derive(Clone, Serialize, Deserialize)]
pub struct MoveStep {
    pub pane: String,
    pub dir: Dir,
    pub target: String,
    pub ratio: f64,
}

pub struct RebuildPlan {
    pub anchor: String,
    pub steps: Vec<MoveStep>,
}

const GAP: u32 = 2;

fn bounds(rects: &[PaneRect]) -> (u32, u32, u32, u32) {
    let x0 = rects.iter().map(|rect| rect.x).min().unwrap();
    let y0 = rects.iter().map(|rect| rect.y).min().unwrap();
    let x1 = rects.iter().map(|rect| rect.x + rect.w).max().unwrap();
    let y1 = rects.iter().map(|rect| rect.y + rect.h).max().unwrap();
    (x0, y0, x1, y1)
}

/// A cut is only valid when the two groups it produces do not geometrically
/// overlap — zero tolerance, or the reconstruction would silently drop the
/// overlap.
fn is_clean_cut(before_max_end: u32, after_min_start: u32) -> bool {
    after_min_start >= before_max_end
}

fn x_start(rect: &PaneRect) -> u32 {
    rect.x
}
fn x_extent(rect: &PaneRect) -> u32 {
    rect.w
}
fn y_start(rect: &PaneRect) -> u32 {
    rect.y
}
fn y_extent(rect: &PaneRect) -> u32 {
    rect.h
}

/// Find a cut position along one axis that separates rects into non-empty,
/// non-overlapping groups.
fn cut(
    rects: &[PaneRect],
    lo: u32,
    hi: u32,
    start: fn(&PaneRect) -> u32,
    extent: fn(&PaneRect) -> u32,
) -> Option<u32> {
    let end = |rect: &PaneRect| start(rect) + extent(rect);
    let mut edges: Vec<u32> = rects
        .iter()
        .map(end)
        .filter(|&edge| edge > lo + GAP && edge + GAP < hi)
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges.into_iter().find(|&cut| {
        let before_max_end = rects.iter().filter(|rect| end(rect) <= cut).map(end).max();
        let after_min_start = rects.iter().filter(|rect| end(rect) > cut).map(start).min();
        match (before_max_end, after_min_start) {
            (Some(before_max_end), Some(after_min_start)) => {
                is_clean_cut(before_max_end, after_min_start)
            }
            _ => false,
        }
    })
}

fn partition(rects: &[PaneRect]) -> Result<(String, Vec<MoveStep>)> {
    if rects.len() == 1 {
        return Ok((rects[0].pane_id.clone(), vec![]));
    }

    let (x0, y0, x1, y1) = bounds(rects);
    type Axis = (Dir, u32, u32, fn(&PaneRect) -> u32, fn(&PaneRect) -> u32);
    let axes: [Axis; 2] = [
        (Dir::Right, x0, x1, x_start, x_extent),
        (Dir::Down, y0, y1, y_start, y_extent),
    ];
    for (dir, lo, hi, start, extent) in axes {
        if let Some(cut_pos) = cut(rects, lo, hi, start, extent) {
            let (first, second): (Vec<_>, Vec<_>) = rects
                .iter()
                .cloned()
                .partition(|rect| start(rect) + extent(rect) <= cut_pos);
            let ratio = (cut_pos - lo) as f64 / (hi - lo) as f64;
            return combine(first, second, dir, ratio);
        }
    }

    bail!(
        "layout is not guillotine-partitionable ({} rects)",
        rects.len()
    )
}

fn combine(
    first: Vec<PaneRect>,
    second: Vec<PaneRect>,
    dir: Dir,
    ratio: f64,
) -> Result<(String, Vec<MoveStep>)> {
    let (first_head, first_steps) = partition(&first)?;
    let (second_head, second_steps) = partition(&second)?;
    let mut steps = vec![MoveStep {
        pane: second_head.clone(),
        dir,
        target: first_head.clone(),
        ratio,
    }];
    steps.extend(first_steps);
    steps.extend(second_steps);
    Ok((first_head, steps))
}

pub fn plan_rebuild(rects: &[PaneRect]) -> Result<RebuildPlan> {
    if rects.is_empty() {
        bail!("no panes");
    }
    let (anchor, steps) = partition(rects)?;
    Ok(RebuildPlan { anchor, steps })
}

// ---- `herdr pane layout` reply parsing ------------------------------------

/// Extract normalized pane rects from a `herdr pane layout` JSON reply.
pub fn parse_pane_rects(value: &Value) -> Result<Vec<PaneRect>> {
    let layout = value
        .pointer("/result/layout")
        .context("pane layout response missing result.layout")?;
    let origin_x = u32_at(layout, "/area/x")?;
    let origin_y = u32_at(layout, "/area/y")?;
    let panes = layout
        .pointer("/panes")
        .and_then(Value::as_array)
        .context("pane layout response missing result.layout.panes array")?;

    panes
        .iter()
        .map(|pane| {
            let x = u32_at(pane, "/rect/x")?;
            let y = u32_at(pane, "/rect/y")?;
            Ok(PaneRect {
                pane_id: pane
                    .pointer("/pane_id")
                    .and_then(Value::as_str)
                    .context("pane layout entry missing pane_id")?
                    .to_owned(),
                x: x.checked_sub(origin_x)
                    .context("pane rect x is outside the layout area")?,
                y: y.checked_sub(origin_y)
                    .context("pane rect y is outside the layout area")?,
                w: u32_at(pane, "/rect/width")?,
                h: u32_at(pane, "/rect/height")?,
            })
        })
        .collect()
}

fn u32_at(value: &Value, pointer: &str) -> Result<u32> {
    let number = value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .with_context(|| format!("JSON response missing unsigned integer at {pointer}"))?;
    number
        .try_into()
        .with_context(|| format!("integer at {pointer} does not fit in u32"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, x: u32, y: u32, w: u32, h: u32) -> PaneRect {
        PaneRect {
            pane_id: id.into(),
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn single_pane_plan_is_anchor_only() {
        let p = plan_rebuild(&[r("p1", 0, 0, 100, 50)]).unwrap();
        assert_eq!(p.anchor, "p1");
        assert!(p.steps.is_empty());
    }

    #[test]
    fn two_columns() {
        let p = plan_rebuild(&[r("a", 0, 0, 40, 50), r("b", 41, 0, 59, 50)]).unwrap();
        assert_eq!(p.anchor, "a");
        assert_eq!(p.steps.len(), 1);
        let s = &p.steps[0];
        assert_eq!((s.pane.as_str(), s.target.as_str()), ("b", "a"));
        assert!(matches!(s.dir, Dir::Right));
        assert!((s.ratio - 0.4).abs() < 0.03);
    }

    #[test]
    fn asymmetric_three_pane() {
        let p = plan_rebuild(&[
            r("a", 0, 0, 40, 52),
            r("b", 41, 0, 59, 15),
            r("c", 41, 16, 59, 36),
        ])
        .unwrap();
        assert_eq!(p.anchor, "a");
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[0].pane, "b");
        assert!(matches!(p.steps[0].dir, Dir::Right));
        assert_eq!(p.steps[1].pane, "c");
        assert_eq!(p.steps[1].target, "b");
        assert!(matches!(p.steps[1].dir, Dir::Down));
        assert!((p.steps[1].ratio - 0.3).abs() < 0.05);
    }

    #[test]
    fn grid_2x2() {
        let p = plan_rebuild(&[
            r("a", 0, 0, 50, 25),
            r("b", 51, 0, 49, 25),
            r("c", 0, 26, 50, 26),
            r("d", 51, 26, 49, 26),
        ])
        .unwrap();
        assert_eq!(p.steps.len(), 3);
    }

    #[test]
    fn overlapping_rects_error() {
        assert!(plan_rebuild(&[r("a", 0, 0, 60, 50), r("b", 30, 0, 70, 50)]).is_err());
        assert!(plan_rebuild(&[r("a", 0, 0, 50, 50), r("b", 49, 0, 51, 50)]).is_err());
    }

    #[test]
    fn parses_pane_layout_reply() {
        let value: Value = serde_json::from_str(
            r#"{"result":{"layout":{"area":{"x":2,"y":1,"width":100,"height":50},
                "panes":[
                  {"pane_id":"w0:p1","rect":{"x":2,"y":1,"width":40,"height":50}},
                  {"pane_id":"w0:p2","rect":{"x":43,"y":1,"width":59,"height":50}}
                ]}}}"#,
        )
        .unwrap();
        let rects = parse_pane_rects(&value).unwrap();
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], r("w0:p1", 0, 0, 40, 50));
        assert_eq!(rects[1], r("w0:p2", 41, 0, 59, 50));
    }
}
