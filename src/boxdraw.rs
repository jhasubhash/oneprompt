//! Box-drawing (U+2500..) and block elements (U+2580..) drawn as rectangles
//! instead of font glyphs. Fonts rarely size these to fill the cell exactly,
//! which leaves seams between cells in TUI borders, progress bars and charts.
//!
//! Also the round dots programs use as bullets (Claude Code starts its
//! replies with ⏺): text fonts lack some of them, and the fallback is a
//! colour emoji, a glossy button on the page. Drawn here, a dot is ink like
//! the text around it.

/// Weight of one arm of a box-drawing character.
#[derive(Clone, Copy, PartialEq, Eq)]
enum W {
    None,
    Light,
    Heavy,
    Double,
}

use W::{Double as D, Heavy as H, Light as L, None as N};

/// Arms of a line character: up, right, down, left.
fn arms(c: char) -> Option<[W; 4]> {
    Some(match c {
        '─' => [N, L, N, L],
        '━' => [N, H, N, H],
        '│' => [L, N, L, N],
        '┃' => [H, N, H, N],
        '┌' | '╭' => [N, L, L, N],
        '┐' | '╮' => [N, N, L, L],
        '└' | '╰' => [L, L, N, N],
        '┘' | '╯' => [L, N, N, L],
        '┏' => [N, H, H, N],
        '┓' => [N, N, H, H],
        '┗' => [H, H, N, N],
        '┛' => [H, N, N, H],
        '├' => [L, L, L, N],
        '┤' => [L, N, L, L],
        '┬' => [N, L, L, L],
        '┴' => [L, L, N, L],
        '┼' => [L, L, L, L],
        '┣' => [H, H, H, N],
        '┫' => [H, N, H, H],
        '┳' => [N, H, H, H],
        '┻' => [H, H, N, H],
        '╋' => [H, H, H, H],
        '═' => [N, D, N, D],
        '║' => [D, N, D, N],
        '╔' => [N, D, D, N],
        '╗' => [N, N, D, D],
        '╚' => [D, D, N, N],
        '╝' => [D, N, N, D],
        '╠' => [D, D, D, N],
        '╣' => [D, N, D, D],
        '╦' => [N, D, D, D],
        '╩' => [D, D, N, D],
        '╬' => [D, D, D, D],
        '╴' => [N, N, N, L],
        '╵' => [L, N, N, N],
        '╶' => [N, L, N, N],
        '╷' => [N, N, L, N],
        '╸' => [N, N, N, H],
        '╹' => [H, N, N, N],
        '╺' => [N, H, N, N],
        '╻' => [N, N, H, N],
        _ => return None,
    })
}

/// A rectangle relative to the cell, plus opacity: (x, y, w, h, alpha).
pub type Piece = (f32, f32, f32, f32, f32);

/// Returns the pieces that draw `c` in a `w` x `h` cell, or `None` if the
/// character should be left to the font.
pub fn pieces(c: char, w: f32, h: f32) -> Option<Vec<Piece>> {
    if let Some(block) = block(c, w, h) {
        return Some(block);
    }
    if let Some(r) = dot(c) {
        return Some(disc(w, h, w.min(h) * r));
    }
    let arms = arms(c)?;

    let t = (w * 0.12).round().max(1.0);
    let (cx, cy) = ((w / 2.0).round(), (h / 2.0).round());
    let mut out = Vec::new();
    // Horizontal arm from x0 to x1 and vertical arm from y0 to y1, centred
    // on the cell's middle line; double arms are two thin parallel lines.
    let mut horizontal = |weight: W, x0: f32, x1: f32| match weight {
        W::None => {}
        W::Light => out.push((x0, cy - t / 2.0, x1 - x0, t, 1.0)),
        W::Heavy => out.push((x0, cy - t, x1 - x0, t * 2.0, 1.0)),
        W::Double => {
            out.push((x0, cy - t * 1.5, x1 - x0, t, 1.0));
            out.push((x0, cy + t * 0.5, x1 - x0, t, 1.0));
        }
    };
    let [up, right, down, left] = arms;
    // Arms overlap the centre by half a stroke so corners close cleanly.
    let reach = |weight: W| if weight == W::Light { t / 2.0 } else { t * 1.5 };
    let (v_reach, h_reach) = (reach(up.max_with(down)), reach(left.max_with(right)));
    horizontal(left, 0.0, cx + v_reach);
    horizontal(right, cx - v_reach, w);
    let mut vertical = |weight: W, y0: f32, y1: f32| match weight {
        W::None => {}
        W::Light => out.push((cx - t / 2.0, y0, t, y1 - y0, 1.0)),
        W::Heavy => out.push((cx - t, y0, t * 2.0, y1 - y0, 1.0)),
        W::Double => {
            out.push((cx - t * 1.5, y0, t, y1 - y0, 1.0));
            out.push((cx + t * 0.5, y0, t, y1 - y0, 1.0));
        }
    };
    vertical(up, 0.0, cy + h_reach);
    vertical(down, cy - h_reach, h);
    Some(out)
}

impl W {
    fn max_with(self, other: W) -> W {
        let rank = |w: W| match w {
            W::None => 0,
            W::Light => 1,
            W::Heavy => 2,
            W::Double => 3,
        };
        if rank(self) >= rank(other) { self } else { other }
    }
}

/// Block elements: halves, eighths, shades and quadrants.
fn block(c: char, w: f32, h: f32) -> Option<Vec<Piece>> {
    let eighth_h = h / 8.0;
    let eighth_w = w / 8.0;
    let (hw, hh) = (w / 2.0, h / 2.0);
    let full = (0.0, 0.0, w, h, 1.0);
    Some(match c {
        '▀' => vec![(0.0, 0.0, w, hh, 1.0)],
        // Lower one eighth (▁) through full block (█).
        '▁'..='█' => {
            let n = (c as u32 - '▁' as u32 + 1) as f32;
            vec![(0.0, h - n * eighth_h, w, n * eighth_h, 1.0)]
        }
        // Left seven eighths (▉) down to left one eighth (▏).
        '▉'..='▏' => {
            let n = (8 - (c as u32 - '▉' as u32 + 1)) as f32;
            vec![(0.0, 0.0, n * eighth_w, h, 1.0)]
        }
        '▐' => vec![(hw, 0.0, w - hw, h, 1.0)],
        '░' => vec![(full.0, full.1, full.2, full.3, 0.25)],
        '▒' => vec![(full.0, full.1, full.2, full.3, 0.5)],
        '▓' => vec![(full.0, full.1, full.2, full.3, 0.75)],
        '▔' => vec![(0.0, 0.0, w, eighth_h, 1.0)],
        '▕' => vec![(w - eighth_w, 0.0, eighth_w, h, 1.0)],
        '▖'..='▟' => {
            // Quadrants as bits: upper-left, upper-right, lower-left, lower-right.
            let bits: u8 = match c {
                '▖' => 0b0010,
                '▗' => 0b0001,
                '▘' => 0b1000,
                '▙' => 0b1011,
                '▚' => 0b1001,
                '▛' => 0b1110,
                '▜' => 0b1101,
                '▝' => 0b0100,
                '▞' => 0b0110,
                _ => 0b0111, // ▟
            };
            let quads = [
                (0.0, 0.0, hw, hh, 1.0),
                (hw, 0.0, w - hw, hh, 1.0),
                (0.0, hh, hw, h - hh, 1.0),
                (hw, hh, w - hw, h - hh, 1.0),
            ];
            (0..4)
                .filter(|i| bits & (0b1000 >> i) != 0)
                .map(|i| quads[i])
                .collect()
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bullets_are_round_dots_in_the_cell() {
        let dot = pieces('⏺', 10.0, 20.0).unwrap();
        assert!(dot.len() >= 7);
        // Widest in the middle, inside the cell, centred.
        let widest = dot.iter().fold(0.0f32, |m, p| m.max(p.2));
        assert!((widest - 7.2).abs() < 0.8, "{widest}");
        assert!(dot.iter().all(|p| p.0 >= 0.0 && p.0 + p.2 <= 10.0 && p.1 >= 0.0 && p.1 < 20.0));
        assert!(pieces('•', 10.0, 20.0).unwrap().len() < dot.len());
        assert!(pieces('a', 10.0, 20.0).is_none());
    }

    fn covers(pieces: &[Piece], x: f32, y: f32) -> bool {
        pieces.iter().any(|&(px, py, pw, ph, _)| x >= px && x < px + pw && y >= py && y < py + ph)
    }

    #[test]
    fn full_block_fills_cell() {
        assert_eq!(pieces('█', 10.0, 20.0).unwrap(), vec![(0.0, 0.0, 10.0, 20.0, 1.0)]);
    }

    #[test]
    fn horizontal_line_reaches_both_edges() {
        let p = pieces('─', 10.0, 20.0).unwrap();
        assert!(covers(&p, 0.0, 10.0) && covers(&p, 9.9, 10.0));
        assert!(!covers(&p, 5.0, 0.0));
    }

    #[test]
    fn corner_joins_its_two_arms() {
        // ┌ goes right and down, never left or up.
        let p = pieces('┌', 10.0, 20.0).unwrap();
        assert!(covers(&p, 9.9, 10.0) && covers(&p, 5.0, 19.9));
        assert!(!covers(&p, 0.0, 10.0) && !covers(&p, 5.0, 0.0));
    }

    #[test]
    fn quadrants_and_eighths() {
        let p = pieces('▚', 10.0, 20.0).unwrap();
        assert!(covers(&p, 1.0, 1.0) && covers(&p, 9.0, 19.0));
        assert!(!covers(&p, 9.0, 1.0) && !covers(&p, 1.0, 19.0));
        let lower = pieces('▄', 10.0, 16.0).unwrap();
        assert_eq!(lower, vec![(0.0, 8.0, 10.0, 8.0, 1.0)]);
        let left = pieces('▎', 16.0, 20.0).unwrap();
        assert_eq!(left, vec![(0.0, 0.0, 4.0, 20.0, 1.0)]);
    }

    #[test]
    fn letters_are_left_to_the_font() {
        assert!(pieces('a', 10.0, 20.0).is_none());
    }
}

/// The dots drawn as discs, and their radius as a share of the cell's width.
fn dot(c: char) -> Option<f32> {
    match c {
        '⏺' | '●' | '⬤' => Some(0.36),
        '•' | '∙' | '⦁' => Some(0.2),
        _ => None,
    }
}

/// A disc of radius `r` in the middle of a `w` x `h` cell, a strip per pixel
/// row, the edge rows faded for a soft rim.
fn disc(w: f32, h: f32, r: f32) -> Vec<Piece> {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let rows = (r * 2.0).ceil() as i32;
    let mut out = Vec::new();
    for k in 0..=rows {
        let y = (cy - r).floor() + k as f32;
        let dy = (y + 0.5 - cy).abs();
        if dy > r + 0.5 {
            continue;
        }
        let half = (r * r - (dy - 0.5).max(0.0).powi(2)).max(0.0).sqrt();
        let alpha = (r + 0.5 - dy).clamp(0.0, 1.0);
        out.push((cx - half, y, half * 2.0, 1.0, alpha));
    }
    out
}
