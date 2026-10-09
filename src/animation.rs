//! Bookkeeping for the ink animations (the drawing happens in shaders):
//!
//! - `TextAnimator` remembers when each visible cell was last written, so the
//!   ink shader can sweep new characters in with the brush and let their bleed
//!   spread as they dry. Text that merely moved (scrolling) keeps its age, and
//!   big repaints (clear, a full-screen app redrawing) appear at once instead of
//!   animating the whole screen.
//! - `Transition` is a brush reveal of the whole window: on startup, and
//!   between the light and dark variants.

/// Birth time far enough in the past that the cell counts as dry.
pub const SETTLED: f32 = -1.0e6;

/// More than this share of cells changing at once is a repaint, not writing.
const REPAINT_SHARE: f32 = 0.33;

#[derive(Default)]
pub struct TextAnimator {
    columns: usize,
    rows: Vec<Row>,
}

#[derive(Clone)]
struct Row {
    hash: u64,
    chars: Vec<char>,
    births: Vec<f32>,
}

impl TextAnimator {
    /// Takes the visible grid (`rows` of `columns` chars, row-major) and the
    /// current time; returns each cell's birth time, row-major.
    pub fn update(&mut self, columns: usize, chars: &[char], now: f32, enabled: bool) -> Vec<f32> {
        let lines = chars.len() / columns.max(1);
        let current: Vec<&[char]> = chars.chunks(columns.max(1)).take(lines).collect();
        let fresh = !enabled || self.columns != columns || self.rows.is_empty();

        let mut rows: Vec<Row> = current
            .iter()
            .map(|cells| Row {
                hash: hash_row(cells),
                chars: cells.to_vec(),
                births: vec![SETTLED; columns],
            })
            .collect();

        if !fresh {
            let shift = scroll_shift(&self.rows, &rows);
            let mut changed = 0usize;
            for (i, row) in rows.iter_mut().enumerate() {
                let before = self.rows.get(i + shift);
                for col in 0..columns {
                    let c = row.chars[col];
                    match before {
                        Some(prev) if prev.chars.get(col) == Some(&c) => {
                            row.births[col] = prev.births[col];
                        }
                        _ if c == ' ' => {}
                        _ => {
                            row.births[col] = now;
                            changed += 1;
                        }
                    }
                }
            }
            if changed as f32 > (columns * lines) as f32 * REPAINT_SHARE {
                for row in &mut rows {
                    for (b, &c) in row.births.iter_mut().zip(&row.chars) {
                        if *b == now && c != ' ' {
                            *b = SETTLED;
                        }
                    }
                }
            }
        }

        self.columns = columns;
        self.rows = rows;
        self.rows.iter().flat_map(|r| r.births.iter().copied()).collect()
    }

    /// The most recent birth on screen, to know whether ink is still drying.
    pub fn latest_birth(&self) -> f32 {
        self.rows
            .iter()
            .flat_map(|r| r.births.iter().copied())
            .fold(SETTLED, f32::max)
    }
}

fn hash_row(cells: &[char]) -> u64 {
    // FNV-1a: cheap and good enough to recognise identical rows.
    cells.iter().fold(0xcbf2_9ce4_8422_2325, |h, &c| {
        (h ^ c as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// How many lines the content moved up since the last frame: the shift that
/// lines up the most identical non-blank rows (0 when nothing moved).
fn scroll_shift(before: &[Row], now: &[Row]) -> usize {
    let blank = hash_row(&vec![' '; now.first().map_or(0, |r| r.chars.len())]);
    let matches = |shift: usize| {
        now.iter()
            .enumerate()
            .filter(|(i, row)| row.hash != blank && before.get(i + shift).is_some_and(|b| b.hash == row.hash))
            .count()
    };
    let best = (1..before.len()).max_by_key(|&s| (matches(s), std::cmp::Reverse(s)));
    match best {
        Some(shift) if matches(shift) > matches(0) => shift,
        _ => 0,
    }
}

/// A brush reveal of the whole window, from the old look to the new one.
#[derive(Clone, Copy, Debug)]
pub struct Transition {
    pub start: f32,
    pub duration: f32,
    /// Where the reveal grows from, in 0..1 window coordinates.
    pub origin: [f32; 2],
    /// What is being revealed over: a solid colour, or the previous frame.
    pub from: From,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum From {
    Color([f32; 3]),
    PreviousFrame,
}

impl Transition {
    /// Eased progress 0..1 at `now`.
    pub fn progress(&self, now: f32) -> f32 {
        let t = ((now - self.start) / self.duration).clamp(0.0, 1.0);
        // Fast start, slow finish, like ink running out on the brush.
        1.0 - (1.0 - t).powi(3)
    }

    pub fn done(&self, now: f32) -> bool {
        now - self.start >= self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(lines: &[&str], cols: usize) -> Vec<char> {
        lines
            .iter()
            .flat_map(|l| {
                let mut v: Vec<char> = l.chars().collect();
                v.resize(cols, ' ');
                v
            })
            .collect()
    }

    #[test]
    fn first_frame_is_settled() {
        let mut a = TextAnimator::default();
        let births = a.update(4, &grid(&["ab", "cd"], 4), 1.0, true);
        assert!(births.iter().all(|&b| b == SETTLED));
    }

    #[test]
    fn typed_characters_get_born() {
        let mut a = TextAnimator::default();
        a.update(4, &grid(&["$ ", "", ""], 4), 1.0, true);
        let births = a.update(4, &grid(&["$ l", "", ""], 4), 2.0, true);
        assert_eq!(births[2], 2.0);
        assert_eq!(births[0], SETTLED);
    }

    #[test]
    fn scrolled_text_keeps_its_age() {
        let mut a = TextAnimator::default();
        a.update(4, &grid(&["aaaa", "bbbb", "cccc"], 4), 1.0, true);
        let b = a.update(4, &grid(&["bbbb", "cccc", "dd"], 4), 5.0, true);
        // Rows that moved up are unchanged; only the new line is fresh.
        assert!(b[..8].iter().all(|&x| x == SETTLED));
        assert_eq!(&b[8..10], &[5.0, 5.0]);
    }

    #[test]
    fn repaints_appear_at_once() {
        let mut a = TextAnimator::default();
        a.update(4, &grid(&["aaaa", "bbbb", "cccc"], 4), 1.0, true);
        let b = a.update(4, &grid(&["wxyz", "wxyz", "wxyz"], 4), 2.0, true);
        assert!(b.iter().all(|&x| x == SETTLED));
    }

    #[test]
    fn transition_progress_is_eased_and_ends() {
        let t = Transition { start: 1.0, duration: 2.0, origin: [0.5, 1.0], from: From::PreviousFrame };
        assert_eq!(t.progress(1.0), 0.0);
        assert!(t.progress(2.0) > 0.5); // ease-out: past halfway at half time
        assert_eq!(t.progress(3.0), 1.0);
        assert!(t.done(3.0) && !t.done(2.9));
    }
}
