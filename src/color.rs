//! Making colours that programs pick themselves (24-bit truecolor and the
//! 256-colour cube) belong on the theme's surface. Programs choose those for
//! a black screen; on parchment light greys vanish and bright yellows glare.
//! So their lightness is moved into the range ink can have on this sheet, in
//! OKLab (perceptual lightness), keeping the hue and most of the colour.
//! (Their background colours become translucent washes in the renderer.)

use crate::theme::Theme;

/// Lightness range text may have, as OKLab L: dark inks on light sheets,
/// light inks on dark ones.
const LIGHT_SHEET: (f32, f32) = (0.18, 0.52);
const DARK_SHEET: (f32, f32) = (0.62, 0.93);
/// Program colours are a little more saturated than inks; tone them down.
const CHROMA: f32 = 0.8;
/// And warm them towards the theme's own ink.
const INK_TINT: f32 = 0.12;

/// A program's text colour, as ink on this theme's surface.
pub fn ink(rgb: [u8; 3], theme: &Theme) -> [u8; 3] {
    let (lo, hi) = if theme.dark { DARK_SHEET } else { LIGHT_SHEET };
    let [l, a, b] = to_oklab(rgb);
    let mapped = [lo + l.clamp(0.0, 1.0) * (hi - lo), a * CHROMA, b * CHROMA];
    let tinted = mix_lab(mapped, to_oklab(theme.ink), INK_TINT);
    from_oklab(tinted)
}

fn mix_lab(x: [f32; 3], y: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| x[i] * (1.0 - t) + y[i] * t)
}

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// sRGB to OKLab (Björn Ottosson's matrices).
pub fn to_oklab(rgb: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(to_linear);
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn from_oklab(lab: [f32; 3]) -> [u8; 3] {
    let [l, a, b] = lab;
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
    .map(from_linear)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin;

    fn lightness(rgb: [u8; 3]) -> f32 {
        to_oklab(rgb)[0]
    }

    #[test]
    fn oklab_round_trips() {
        for rgb in [[0, 0, 0], [255, 255, 255], [215, 119, 87], [40, 200, 64], [12, 34, 200]] {
            let back = from_oklab(to_oklab(rgb));
            for i in 0..3 {
                assert!((back[i] as i32 - rgb[i] as i32).abs() <= 1, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn screen_colours_become_readable_ink() {
        let themes = builtin();
        let light = themes.iter().find(|t| !t.dark).unwrap();
        let dark = themes.iter().find(|t| t.dark).unwrap();
        // Light grey meant for a black screen: dark enough on parchment.
        assert!(lightness(ink([153, 153, 153], light)) <= LIGHT_SHEET.1 + 0.01);
        // Near-black meant for a white screen: light enough on the ink field.
        assert!(lightness(ink([20, 20, 20], dark)) >= DARK_SHEET.0 - 0.01);
        // Order is kept: lighter in, lighter out.
        assert!(lightness(ink([200, 200, 200], light)) > lightness(ink([60, 60, 60], light)));
    }

    #[test]
    fn hue_survives() {
        let light = builtin().into_iter().find(|t| !t.dark).unwrap();
        let [r, g, b] = ink([255, 220, 0], &light); // a warning yellow
        assert!(r > b && g > b, "still yellowish: {r} {g} {b}");
        let [r, g, b] = ink([0, 120, 255], &light); // a link blue
        assert!(b > r, "still bluish: {r} {g} {b}");
    }
}
