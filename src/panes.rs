//! Splits: a tab's panes, each running its own shell, as a tree of
//! splits. Cmd+D splits the pane with the keyboard side by side, Cmd+Shift+D
//! one above the other; Cmd+W closes it. The dividers between panes can be
//! dragged.
//!
//! This file is the tree and its geometry; main.rs keeps a tree per tab
//! and the renderer draws each pane in the box it gets.

use crate::terminal::PaneId;

/// A pane can't be squeezed below this share of its split.
const MIN_RATIO: f32 = 0.1;

/// Called for every part of a layout with its box and the way to it.
type Visit<'a> = dyn FnMut(&Layout, [f32; 4], &[bool]) + 'a;

#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Pane(PaneId),
    /// Two parts, `across` (side by side) or one above the other, the
    /// first taking `ratio` of the space.
    Split { across: bool, ratio: f32, first: Box<Layout>, second: Box<Layout> },
}

/// A divider between the two parts of a split: the line between them, the
/// split's box, and the way to the split from the top (false: first part).
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub line: [f32; 4],
    pub across: bool,
    pub split: [f32; 4],
    pub path: Vec<bool>,
}

impl Layout {
    /// Every pane's box inside `rect` (x, y, w, h), in reading order.
    pub fn rects(&self, rect: [f32; 4]) -> Vec<(PaneId, [f32; 4])> {
        let mut out = Vec::new();
        self.walk(rect, &mut Vec::new(), &mut |layout, rect, _| {
            if let Layout::Pane(id) = layout {
                out.push((*id, rect));
            }
        });
        out
    }

    /// The dividers inside `rect`, each `width` wide (for drawing, and to
    /// grab).
    pub fn dividers(&self, rect: [f32; 4], width: f32) -> Vec<Divider> {
        let mut out = Vec::new();
        self.walk(rect, &mut Vec::new(), &mut |layout, rect, path| {
            if let Layout::Split { across, ratio, .. } = layout {
                let [x, y, w, h] = rect;
                let line = if *across {
                    [x + (w * ratio).round() - width / 2.0, y, width, h]
                } else {
                    [x, y + (h * ratio).round() - width / 2.0, w, width]
                };
                out.push(Divider { line, across: *across, split: rect, path: path.to_vec() });
            }
        });
        out
    }

    fn walk(&self, rect: [f32; 4], path: &mut Vec<bool>, visit: &mut Visit) {
        visit(self, rect, path);
        if let Layout::Split { across, ratio, first, second } = self {
            let [x, y, w, h] = rect;
            let (a, b) = if *across {
                let left = (w * ratio).round();
                ([x, y, left, h], [x + left, y, w - left, h])
            } else {
                let top = (h * ratio).round();
                ([x, y, w, top], [x, y + top, w, h - top])
            };
            path.push(false);
            first.walk(a, path, visit);
            path.pop();
            path.push(true);
            second.walk(b, path, visit);
            path.pop();
        }
    }

    /// Splits pane `id` in two, `new` after it. Returns false if there is
    /// no such pane.
    pub fn split(&mut self, id: PaneId, new: PaneId, across: bool) -> bool {
        match self {
            Layout::Pane(p) if *p == id => {
                *self = Layout::Split {
                    across,
                    ratio: 0.5,
                    first: Box::new(Layout::Pane(id)),
                    second: Box::new(Layout::Pane(new)),
                };
                true
            }
            Layout::Pane(_) => false,
            Layout::Split { first, second, .. } => first.split(id, new, across) || second.split(id, new, across),
        }
    }

    /// Takes pane `id` out; the other part of its split takes its place.
    /// Returns false if it is not here, or is the only pane.
    pub fn remove(&mut self, id: PaneId) -> bool {
        let Layout::Split { first, second, .. } = self else { return false };
        if **first == Layout::Pane(id) {
            *self = (**second).clone();
            return true;
        }
        if **second == Layout::Pane(id) {
            *self = (**first).clone();
            return true;
        }
        first.remove(id) || second.remove(id)
    }

    /// Sets the split at `path` to give its first part `ratio`, within
    /// limits so neither part disappears.
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        match (self, path.split_first()) {
            (Layout::Split { ratio: r, .. }, None) => *r = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO),
            (Layout::Split { first, second, .. }, Some((&side, rest))) => {
                if side { second } else { first }.set_ratio(rest, ratio)
            }
            _ => {}
        }
    }

    /// The panes, in reading order.
    pub fn ids(&self) -> Vec<PaneId> {
        self.rects([0.0, 0.0, 1.0, 1.0]).into_iter().map(|(id, _)| id).collect()
    }
}

/// The pane next to `from` in direction (dx, dy) (one of them ±1): of the
/// panes beyond its edge that side and level with it, the nearest.
pub fn neighbour(rects: &[(PaneId, [f32; 4])], from: PaneId, (dx, dy): (i32, i32)) -> Option<PaneId> {
    let &(_, [x, y, w, h]) = rects.iter().find(|(id, _)| *id == from)?;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    rects
        .iter()
        .filter(|&&(id, [ox, oy, ow, oh])| {
            let beyond = match (dx, dy) {
                (1, _) => ox >= x + w - 1.0,
                (-1, _) => ox + ow <= x + 1.0,
                (_, 1) => oy >= y + h - 1.0,
                _ => oy + oh <= y + 1.0,
            };
            let level = if dx != 0 { oy < y + h && oy + oh > y } else { ox < x + w && ox + ow > x };
            id != from && beyond && level
        })
        .min_by(|a, b| {
            let d = |r: &[f32; 4]| (r[0] + r[2] / 2.0 - cx).abs() + (r[1] + r[3] / 2.0 - cy).abs();
            d(&a.1).total_cmp(&d(&b.1))
        })
        .map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_share_the_box_and_close_back() {
        let mut layout = Layout::Pane(1);
        assert!(layout.split(1, 2, true));
        assert!(layout.split(2, 3, false));
        assert!(!layout.split(9, 4, true));
        let rects = layout.rects([0.0, 0.0, 100.0, 60.0]);
        assert_eq!(rects, vec![(1, [0.0, 0.0, 50.0, 60.0]), (2, [50.0, 0.0, 50.0, 30.0]), (3, [50.0, 30.0, 50.0, 30.0])]);
        assert_eq!(layout.ids(), vec![1, 2, 3]);

        assert_eq!(neighbour(&rects, 1, (1, 0)), Some(2));
        assert_eq!(neighbour(&rects, 3, (-1, 0)), Some(1));
        assert_eq!(neighbour(&rects, 2, (0, 1)), Some(3));
        assert_eq!(neighbour(&rects, 2, (0, -1)), None);

        let dividers = layout.dividers([0.0, 0.0, 100.0, 60.0], 4.0);
        assert_eq!(dividers.len(), 2);
        assert_eq!(dividers[0].line, [48.0, 0.0, 4.0, 60.0]);
        assert_eq!(dividers[1].path, vec![true]);
        layout.set_ratio(&dividers[1].path, 0.0);
        assert_eq!(layout.rects([0.0, 0.0, 100.0, 60.0])[1].1, [50.0, 0.0, 50.0, 6.0]);

        assert!(layout.remove(2));
        assert_eq!(layout.rects([0.0, 0.0, 100.0, 60.0]), vec![(1, [0.0, 0.0, 50.0, 60.0]), (3, [50.0, 0.0, 50.0, 60.0])]);
        assert!(layout.remove(1));
        assert_eq!(layout, Layout::Pane(3));
        assert!(!layout.remove(3));
    }
}
