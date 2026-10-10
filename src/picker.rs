//! The theme picker (Cmd+Option+T, or Themes… in the menu): every theme as
//! a card showing the current screen as that theme draws it, live. Arrow
//! keys or the pointer choose, Return or a click applies (with the theme's
//! own switch), Esc or a click outside closes. Cmd+Shift+L still flips light
//! and dark, so both variants can be looked at.
//!
//! This file is the picker's state and geometry; the renderer draws it
//! (`Renderer::set_picker`) and main.rs routes keys and clicks to it.

/// Physical-pixel boxes (x, y, w, h) of the cards, for `n` themes in a
/// `w` x `h` window at `scale`: one row, centred, each a small copy of the
/// window, with room below for its name and description.
pub fn cards(n: usize, w: f32, h: f32, scale: f32) -> Vec<[f32; 4]> {
    if n == 0 {
        return Vec::new();
    }
    let margin = 40.0 * scale;
    let gap = 28.0 * scale;
    let top = h * 0.2;
    let below = 64.0 * scale;
    let bottom = h - 64.0 * scale;
    let aspect = w / h.max(1.0);
    let (avail_w, avail_h) = (w - 2.0 * margin, (bottom - top - below).max(1.0));
    let mut card_w = ((avail_w - gap * (n as f32 - 1.0)) / n as f32).min(w * 0.42).max(1.0);
    let mut card_h = card_w / aspect;
    if card_h > avail_h {
        card_h = avail_h;
        card_w = card_h * aspect;
    }
    let total = card_w * n as f32 + gap * (n as f32 - 1.0);
    let x0 = (w - total) / 2.0;
    (0..n).map(|i| [x0 + i as f32 * (card_w + gap), top, card_w, card_h]).collect()
}

/// The card under (x, y), counting the label below it.
pub fn card_at(cards: &[[f32; 4]], x: f32, y: f32, scale: f32) -> Option<usize> {
    let below = 64.0 * scale;
    cards.iter().position(|&[cx, cy, cw, ch]| x >= cx && x < cx + cw && y >= cy && y < cy + ch + below)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_fit_the_window_side_by_side() {
        for &(w, h) in &[(960.0, 600.0), (1920.0, 1200.0), (500.0, 900.0), (2400.0, 500.0)] {
            for n in 1..5 {
                let cards = cards(n, w, h, 2.0);
                assert_eq!(cards.len(), n);
                for (i, &[x, y, cw, ch]) in cards.iter().enumerate() {
                    assert!(x >= 0.0 && x + cw <= w + 0.5 && y >= 0.0 && y + ch <= h, "{w}x{h} n={n}: {:?}", cards[i]);
                    // Same shape as the window, so the preview is not squashed.
                    assert!((cw / ch - w / h).abs() < 0.01);
                    if i > 0 {
                        assert!(cards[i - 1][0] + cards[i - 1][2] < x);
                    }
                }
                let [x, y, cw, ch] = cards[0];
                assert_eq!(card_at(&cards, x + cw / 2.0, y + ch / 2.0, 2.0), Some(0));
                assert_eq!(card_at(&cards, 1.0, 1.0, 2.0), None);
            }
        }
    }
}
