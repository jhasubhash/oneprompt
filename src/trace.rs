//! Pen paths for the writing animation: for every glyph, the order in which a
//! pen tracing it would reach each pixel, so the ink shader can reveal a
//! character along its strokes instead of sweeping across its cell.
//!
//! Fonts carry no stroke order, so it is derived from the glyph's shape:
//! - the glyph is split into connected pieces (the dot of an i, the two parts
//!   of a quote), written left to right;
//! - each piece is traced from its top-left-most point, and a pixel's order is
//!   its distance *along the ink* from there (a geodesic, so the front flows
//!   around bowls and down stems rather than across the cell);
//! - pixels outside the ink (antialiasing, bleed) take the order of the
//!   nearest ink, so edges appear with their stroke.
//!
//! Orders are stored 1..=255 in an R8 atlas, one cell-sized slot per glyph;
//! 0 means "no path", and the shader falls back to a brush sweep.

use std::collections::{BinaryHeap, HashMap, VecDeque};

/// Coverage above which a pixel counts as ink.
const INK: u8 = 80;
/// Pause between pieces, as a share of the glyph's total path length.
const LIFT: f32 = 0.08;

/// Pen order for a `w` x `h` coverage bitmap: 0 for empty pixels far from any
/// ink, otherwise 1..=255 along the path.
pub fn pen_order(alpha: &[u8], w: usize, h: usize) -> Vec<u8> {
    let ink: Vec<bool> = alpha.iter().map(|&a| a >= INK).collect();
    let mut component = vec![usize::MAX; w * h];
    let mut pieces: Vec<Vec<usize>> = Vec::new();

    // Connected pieces (8-neighbourhood).
    for start in 0..w * h {
        if !ink[start] || component[start] != usize::MAX {
            continue;
        }
        let id = pieces.len();
        let mut members = Vec::new();
        let mut queue = VecDeque::from([start]);
        component[start] = id;
        while let Some(i) = queue.pop_front() {
            members.push(i);
            for n in neighbours(i, w, h) {
                if ink[n] && component[n] == usize::MAX {
                    component[n] = id;
                    queue.push_back(n);
                }
            }
        }
        pieces.push(members);
    }
    if pieces.is_empty() {
        return vec![0; w * h];
    }

    // Written left to right (then top to bottom), like a hand would.
    let key = |p: &Vec<usize>| {
        let x = p.iter().map(|&i| i % w).min().unwrap_or(0);
        let y = p.iter().map(|&i| i / w).min().unwrap_or(0);
        (x / 2, y)
    };
    pieces.sort_by_key(key);

    // Geodesic distance inside each piece from its top-left-most pixel.
    let mut dist = vec![f32::INFINITY; w * h];
    let mut offset = 0.0f32;
    let mut lengths = Vec::new();
    for piece in &pieces {
        let start = *piece
            .iter()
            .min_by(|&&a, &&b| {
                let score = |i: usize| (i % w) as f32 + (i / w) as f32 * 0.8;
                score(a).total_cmp(&score(b))
            })
            .expect("piece is not empty");
        let longest = geodesic(&ink, w, h, start, &mut dist);
        lengths.push((offset, longest));
        offset += longest;
    }
    // Shift each piece by the length of the ones before it, plus a pen lift.
    let total = offset.max(1.0);
    let lift = total * LIFT;
    let mut order = vec![0.0f32; w * h];
    for (k, piece) in pieces.iter().enumerate() {
        let start = lengths[k].0 + lift * k as f32;
        for &i in piece {
            order[i] = start + dist[i];
        }
    }
    let span = total + lift * (pieces.len() - 1) as f32;

    // Antialiased and empty pixels take the order of the nearest ink.
    let mut out = vec![0u8; w * h];
    let mut queue = VecDeque::new();
    let mut seen = vec![false; w * h];
    for i in 0..w * h {
        if ink[i] {
            out[i] = to_u8(order[i] / span);
            seen[i] = true;
            queue.push_back((i, 0u32));
        }
    }
    while let Some((i, steps)) = queue.pop_front() {
        // Only a few pixels out: enough for edges and a little bleed.
        if steps >= 3 {
            continue;
        }
        for n in neighbours(i, w, h) {
            if !seen[n] {
                seen[n] = true;
                out[n] = out[i];
                queue.push_back((n, steps + 1));
            }
        }
    }
    out
}

fn to_u8(t: f32) -> u8 {
    (1.0 + t.clamp(0.0, 1.0) * 254.0).round() as u8
}

fn neighbours(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i % w) as isize, (i / w) as isize);
    (-1..=1)
        .flat_map(move |dy| (-1..=1).map(move |dx| (dx, dy)))
        .filter(|&(dx, dy)| dx != 0 || dy != 0)
        .filter_map(move |(dx, dy)| {
            let (nx, ny) = (x + dx, y + dy);
            (nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h)
                .then(|| ny as usize * w + nx as usize)
        })
}

/// Dijkstra over ink pixels from `start`; returns the longest distance found.
fn geodesic(ink: &[bool], w: usize, h: usize, start: usize, dist: &mut [f32]) -> f32 {
    #[derive(PartialEq)]
    struct Item(f32, usize);
    impl Eq for Item {}
    impl PartialOrd for Item {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Item {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            other.0.total_cmp(&self.0) // min-heap
        }
    }
    let mut heap = BinaryHeap::from([Item(0.0, start)]);
    dist[start] = 0.0;
    let mut longest = 0.0f32;
    while let Some(Item(d, i)) = heap.pop() {
        if d > dist[i] {
            continue;
        }
        longest = longest.max(d);
        for n in neighbours(i, w, h) {
            if !ink[n] {
                continue;
            }
            let diagonal = (n % w != i % w) && (n / w != i / w);
            let next = d + if diagonal { std::f32::consts::SQRT_2 } else { 1.0 };
            if next < dist[n] {
                dist[n] = next;
                heap.push(Item(next, n));
            }
        }
    }
    longest
}

/// Hands out atlas slots for glyphs and remembers which are already traced.
pub struct Tracer {
    pub slot_size: (u32, u32),
    pub atlas_size: (u32, u32),
    slots: HashMap<GlyphKey, u32>,
    next: u32,
}

/// What determines a glyph's shape on screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    pub c: char,
    pub weight: u16,
    pub italic: bool,
}

pub enum Slot {
    /// Already traced.
    Ready(u32),
    /// New: trace the glyph and upload its map to this slot.
    New(u32),
    /// Atlas full or glyph unusable: fall back to the sweep.
    None,
}

impl Tracer {
    pub fn new(slot_size: (u32, u32), atlas_size: (u32, u32)) -> Self {
        // Slot 0 is reserved for "no path".
        Self { slot_size, atlas_size, slots: HashMap::new(), next: 1 }
    }

    pub fn columns(&self) -> u32 {
        (self.atlas_size.0 / self.slot_size.0.max(1)).max(1)
    }

    fn capacity(&self) -> u32 {
        self.columns() * (self.atlas_size.1 / self.slot_size.1.max(1))
    }

    pub fn slot(&mut self, key: GlyphKey) -> Slot {
        if let Some(&slot) = self.slots.get(&key) {
            return Slot::Ready(slot);
        }
        if self.next >= self.capacity() {
            return Slot::None;
        }
        let slot = self.next;
        self.next += 1;
        self.slots.insert(key, slot);
        Slot::New(slot)
    }

    /// Marks a glyph as untraceable (e.g. a space), so it is not retried.
    pub fn forget(&mut self, key: GlyphKey) {
        self.slots.insert(key, 0);
    }

    /// Top-left texel of a slot in the atlas.
    pub fn origin(&self, slot: u32) -> (u32, u32) {
        let cols = self.columns();
        ((slot % cols) * self.slot_size.0, (slot / cols) * self.slot_size.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(rows: &[&str]) -> (Vec<u8>, usize, usize) {
        let (w, h) = (rows[0].len(), rows.len());
        let data = rows
            .iter()
            .flat_map(|r| r.bytes().map(|b| if b == b'#' { 255 } else { 0 }))
            .collect();
        (data, w, h)
    }

    #[test]
    fn a_stem_is_traced_from_the_top() {
        let (a, w, h) = bitmap(&["..#..", "..#..", "..#..", "..#.."]);
        let o = pen_order(&a, w, h);
        let stem: Vec<u8> = (0..h).map(|y| o[y * w + 2]).collect();
        assert!(stem.windows(2).all(|p| p[0] < p[1]), "{stem:?}");
        assert_eq!(stem[0], 1);
        assert_eq!(stem[3], 255);
    }

    #[test]
    fn the_path_follows_the_ink_not_the_cell() {
        // A "U": the right arm is reached by going down, across and up, so
        // its top is written last even though it is near the start.
        let (a, w, h) = bitmap(&["#...#", "#...#", "#...#", "#####"]);
        let o = pen_order(&a, w, h);
        assert_eq!(o[0], 1); // top-left start
        assert!(o[4] > o[3 * w + 2], "right arm after the bottom");
        assert_eq!(o[4], 255);
    }

    #[test]
    fn pieces_go_left_to_right_with_a_lift() {
        // Two separate dots: the left one comes first.
        let (a, w, h) = bitmap(&["#...#"]);
        let o = pen_order(&a, w, h);
        assert!(o[0] < o[4]);
    }

    #[test]
    fn edges_take_the_nearest_ink_and_empty_glyphs_have_no_path() {
        let (a, w, h) = bitmap(&[".....", "..#..", "....."]);
        let o = pen_order(&a, w, h);
        assert_eq!(o[0], o[2 * w + 2 - w]); // a neighbour of the dot
        assert!(pen_order(&[0; 6], 3, 2).iter().all(|&v| v == 0));
    }

    #[test]
    fn slots_are_reused_and_bounded() {
        let mut t = Tracer::new((10, 20), (30, 40)); // 3 x 2 = 6 slots, 0 reserved
        let key = |c| GlyphKey { c, weight: 700, italic: false };
        assert!(matches!(t.slot(key('a')), Slot::New(1)));
        assert!(matches!(t.slot(key('a')), Slot::Ready(1)));
        for c in ['b', 'c', 'd', 'e'] {
            assert!(matches!(t.slot(key(c)), Slot::New(_)));
        }
        assert!(matches!(t.slot(key('f')), Slot::None));
        assert_eq!(t.origin(4), (10, 20));
    }
}
