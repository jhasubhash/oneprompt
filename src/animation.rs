//! Bookkeeping for the ink animations (the drawing happens in shaders):
//!
//! - `TextAnimator` remembers when each visible cell was last written, so the
//!   ink shader can sweep new characters in with the brush and let their bleed
//!   spread as they dry. Output arriving together is written out in reading
//!   order, left to right and line by line, at the writing speed. Text that
//!   merely moved (scrolling) keeps its age, and repaints (a full-screen app
//!   redrawing over its own text) appear at once.
//! - `Transition` is a brush reveal of the whole window: on startup, and
//!   between the light and dark variants.

/// Birth time far enough in the past that the cell counts as dry.
pub const SETTLED: f32 = -1.0e6;

/// More than this share of cells being overwritten at once is a repaint.
const REPAINT_SHARE: f32 = 0.33;
/// A line break pauses the pen for this many characters' worth of time.
const LINE_PAUSE: f32 = 3.0;

/// How output is written.
#[derive(Clone, Copy, Debug)]
pub struct Writing {
    pub enabled: bool,
    /// Characters per second when several arrive together.
    pub speed: f32,
    /// The longest one batch of output may take, so the terminal never lags
    /// far behind the program: big outputs are written faster.
    pub max_duration: f32,
    /// Full-screen apps (vim, htop, ...): no writing, everything at once.
    pub instant: bool,
}

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
    /// current time; returns each cell's birth time, row-major. Births may lie
    /// in the future: that cell is waiting for the pen to reach it.
    pub fn update(&mut self, columns: usize, chars: &[char], now: f32, writing: Writing) -> Vec<f32> {
        let lines = chars.len() / columns.max(1);
        let current: Vec<&[char]> = chars.chunks(columns.max(1)).take(lines).collect();
        let fresh = !writing.enabled || self.columns != columns || self.rows.is_empty();

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
            // New ink, in reading order, and how much of it overwrote text.
            let mut written: Vec<(usize, usize)> = Vec::new();
            let mut overwritten = 0usize;
            for (i, row) in rows.iter_mut().enumerate() {
                let before = self.rows.get(i + shift);
                for col in 0..columns {
                    let c = row.chars[col];
                    let old = before.and_then(|prev| prev.chars.get(col).copied());
                    if old == Some(c) {
                        row.births[col] = before.map_or(SETTLED, |prev| prev.births[col]);
                    } else if c != ' ' {
                        written.push((i, col));
                        if old.is_some_and(|o| o != ' ') {
                            overwritten += 1;
                        }
                    }
                }
            }

            let repaint = overwritten as f32 > (columns * lines) as f32 * REPAINT_SHARE;
            if !written.is_empty() && !writing.instant && !repaint {
                // Pen time per character, counting a pause at each new line;
                // squeezed when the batch would take longer than allowed.
                let breaks = written.windows(2).filter(|w| w[0].0 != w[1].0).count() as f32;
                let steps = written.len() as f32 - 1.0 + breaks * LINE_PAUSE;
                let per = (1.0 / writing.speed.max(1.0)).min(writing.max_duration / steps.max(1.0));
                let mut t = now;
                for (k, &(i, col)) in written.iter().enumerate() {
                    if k > 0 {
                        let new_line = written[k - 1].0 != i;
                        t += per * if new_line { 1.0 + LINE_PAUSE } else { 1.0 };
                    }
                    rows[i].births[col] = t;
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
    /// How the brush uncovers the window (see `present.wgsl`).
    pub style: Reveal,
    /// Varies the brush shape, so no two reveals look the same.
    pub shape: f32,
}

/// Ways the new look is brushed in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reveal {
    /// Grows from `origin` in ragged lobes.
    Bloom = 0,
    /// One broad stroke across the window, at some angle.
    Sweep = 1,
    /// A few horizontal strokes, top to bottom, alternating direction.
    Bands = 2,
    /// Several drops of ink landing and spreading into each other.
    Drops = 3,
}

impl Reveal {
    pub const ALL: [Reveal; 4] = [Reveal::Bloom, Reveal::Sweep, Reveal::Bands, Reveal::Drops];
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

    const WRITE: Writing = Writing { enabled: true, speed: 100.0, max_duration: 1.5, instant: false };

    #[test]
    fn first_frame_is_settled() {
        let mut a = TextAnimator::default();
        let births = a.update(4, &grid(&["ab", "cd"], 4), 1.0, WRITE);
        assert!(births.iter().all(|&b| b == SETTLED));
    }

    #[test]
    fn typed_characters_get_born() {
        let mut a = TextAnimator::default();
        a.update(4, &grid(&["$ ", "", ""], 4), 1.0, WRITE);
        let births = a.update(4, &grid(&["$ l", "", ""], 4), 2.0, WRITE);
        assert_eq!(births[2], 2.0);
        assert_eq!(births[0], SETTLED);
    }

    #[test]
    fn scrolled_text_keeps_its_age() {
        let mut a = TextAnimator::default();
        a.update(4, &grid(&["aaaa", "bbbb", "cccc"], 4), 1.0, WRITE);
        let b = a.update(4, &grid(&["bbbb", "cccc", "dd"], 4), 5.0, WRITE);
        // Rows that moved up are unchanged; only the new line is written,
        // left to right at 100 characters per second.
        assert!(b[..8].iter().all(|&x| x == SETTLED));
        assert_eq!(&b[8..10], &[5.0, 5.01]);
    }

    #[test]
    fn repaints_appear_at_once() {
        let mut a = TextAnimator::default();
        a.update(4, &grid(&["aaaa", "bbbb", "cccc"], 4), 1.0, WRITE);
        let b = a.update(4, &grid(&["wxyz", "wxyz", "wxyz"], 4), 2.0, WRITE);
        assert!(b.iter().all(|&x| x == SETTLED));
    }

    #[test]
    fn output_is_written_line_by_line() {
        let mut a = TextAnimator::default();
        a.update(4, &grid(&["$", "", ""], 4), 1.0, WRITE);
        // `ls` printing two lines into blank rows at once.
        let b = a.update(4, &grid(&["$", "ab", "cd"], 4), 2.0, WRITE);
        let (a1, b1, c2, d2) = (b[4], b[5], b[8], b[9]);
        assert_eq!(a1, 2.0);
        assert!(a1 < b1 && b1 < c2 && c2 < d2, "reading order");
        // A line break pauses the pen: more than one character's time.
        assert!(c2 - b1 > (b1 - a1) * 2.0);
    }

    #[test]
    fn big_output_is_squeezed_and_apps_are_instant() {
        let mut a = TextAnimator::default();
        let cols = 80;
        a.update(cols, &grid(&["", "", "", ""], cols), 1.0, WRITE);
        let full = "x".repeat(80);
        let lines = [full.as_str(); 4];
        let b = a.update(cols, &grid(&lines, cols), 2.0, WRITE);
        // 320 characters at 100/s would take 3.2 s; capped at 1.5 s.
        let last = b.iter().copied().fold(SETTLED, f32::max);
        assert!((last - 2.0 - 1.5).abs() < 1e-3, "{last}");

        let mut a = TextAnimator::default();
        a.update(4, &grid(&["", ""], 4), 1.0, WRITE);
        let app = Writing { instant: true, ..WRITE };
        let b = a.update(4, &grid(&["ab", "cd"], 4), 2.0, app);
        assert!(b.iter().all(|&x| x == SETTLED));
    }

    #[test]
    fn transition_progress_is_eased_and_ends() {
        let t = Transition {
            start: 1.0,
            duration: 2.0,
            origin: [0.5, 1.0],
            from: From::PreviousFrame,
            style: Reveal::Bloom,
            shape: 0.0,
        };
        assert_eq!(t.progress(1.0), 0.0);
        assert!(t.progress(2.0) > 0.5); // ease-out: past halfway at half time
        assert_eq!(t.progress(3.0), 1.0);
        assert!(t.done(3.0) && !t.done(2.9));
    }
}
