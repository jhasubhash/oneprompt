//! Tsushima: after the ink-painting cutscenes of Ghost of Tsushima / Yotei.
//! Golden washi darkening to brown at the edges, near-black sumi that bleeds
//! and creeps into the paper, a crimson seal. The dark variant inverts it as
//! the cutscenes do: a field of ink, the drawing showing through in
//! parchment.
//!
//! The window opens with brushwork, switching to Tsushima is a sword fight
//! washed in like watercolour (brush.rs, transition.wgsl), and deleted names
//! are cut through with a sword that flings blood.

mod brush;

use crate::theme::{
    self, CommandSpan, ControlStyle, Controls, Ctx, Cursor, CursorKind, DividerSpan, Param, Plan, Rng, Shape, Shaders,
    Strike, StrikeSpan, Theme, Variant, CUSTOM, DISC,
};

use brush::{Brush, PlanData};

pub static TSUSHIMA: Tsushima = Tsushima;

pub struct Tsushima;

/// Shape kinds drawn by shapes.wgsl.
const DAB: f32 = CUSTOM;
const SLASH: f32 = CUSTOM + 1.0;
const CUT: f32 = CUSTOM + 2.0;
const SEAL: f32 = CUSTOM + 3.0;
/// A seal comes down onto the paper this fast, and shakes this long.
const STAMP_TIME: f32 = 0.22;
const THUD_TIME: f32 = 0.28;
/// A new divider between panes is cut in this fast, at speed 1.
const CUT_TIME: f32 = 0.32;

/// Window reveal on opening and switching, at speed 1.
const REVEAL_TIME: f32 = 1.6;
const SWITCH_TIME: f32 = 2.8;

/// A failed command's stroke is dragged down the margin this fast.
const FAILURE_TIME: f32 = 0.3;

/// How long the sword takes to cut through a deleted name, at speed 1.
const STRIKE_TIME: f32 = 0.22;
/// How far struck text fades towards the paper.
const STRIKE_FADE: f32 = 0.45;
/// Blood flung off the blade: how long until it has dried to a stain (at
/// speed 1, after the cut), and how strong the dried stain stays.
const SPLATTER_DRY: f32 = 2.4;
const SPLATTER_STAIN: f32 = 0.3;

const PARAMS: &[Param] = &[
    Param { key: "bleed", about: "how far ink wicks past strokes (pixels)", min: 0.0, max: 10.0 },
    Param {
        key: "bleed_variation",
        about: "how much the bleed differs per letter, 0..1 (0.35: each letter bleeds 0.65 to 1.35 times `bleed`)",
        min: 0.0,
        max: 1.0,
    },
    Param { key: "bleed_time", about: "seconds the bleed keeps spreading after a letter is written", min: 0.0, max: 30.0 },
    Param { key: "nib", about: "brush nib length: thick-and-thin strokes (pixels, 0 = off)", min: 0.0, max: 10.0 },
    Param { key: "grain", about: "dry-brush streaks, 0..1", min: 0.0, max: 1.0 },
    Param { key: "edge", about: "pigment pooling at stroke rims, 0..1", min: 0.0, max: 1.0 },
    Param {
        key: "absorb",
        about: "how much the ink takes on the paper's grain, 0..1 (0 = ink sits on top)",
        min: 0.0,
        max: 1.0,
    },
    Param { key: "texture", about: "paper texture (mottling, relief, fibers): 1 built-in, 0 flat, up to 3", min: 0.0, max: 3.0 },
    Param { key: "stains", about: "water-stain tide lines: 1 built-in, 0 none, up to 3", min: 0.0, max: 3.0 },
    Param { key: "splatter", about: "ink flecks in the top-right corner: 1 built-in, 0 none, up to 3", min: 0.0, max: 3.0 },
];

/// Per variant: the tone the paper darkens towards, and the colour of the
/// ink flecks in its corner.
fn paper(variant: &Variant) -> ([f32; 3], [u8; 3]) {
    if variant.dark { ([0.2, 0.158, 0.115], [186, 40, 44]) } else { ([0.36, 0.26, 0.14], [168, 30, 36]) }
}

/// What common.wgsl reads (its `Tsushima` struct).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Data {
    shade: [f32; 4],
    splatter: [f32; 4],
    stains: f32,
    bleed: f32,
    grain: f32,
    edge: f32,
    nib: f32,
    bleed_variation: f32,
    bleed_time: f32,
    absorb: f32,
}

impl Theme for Tsushima {
    fn id(&self) -> &'static str {
        "tsushima"
    }

    fn name(&self) -> &'static str {
        "Tsushima"
    }

    fn about(&self) -> &'static str {
        "Sumi ink on hand-made washi, after the ink-painting cutscenes of Ghost of Tsushima."
    }

    fn variants(&'static self) -> Vec<Variant> {
        vec![
            Variant {
                theme: self,
                name: "Tsushima Light",
                dark: false,
                surface: [0.886, 0.769, 0.557],
                ink: [18, 13, 9],
                cursor: [182, 56, 38],
                selection: ([30, 22, 15], 0.9),
                palette: [
                    [22, 20, 18],   // black: sumi
                    [158, 28, 32],  // red: crimson seal
                    [72, 92, 58],   // green: moss
                    [150, 108, 36], // yellow: old gold
                    [38, 56, 92],   // blue: indigo
                    [116, 44, 74],  // magenta: plum
                    [56, 92, 96],   // cyan: weathered teal
                    [102, 92, 80],  // "white": thin ink wash
                    [70, 64, 58],
                    [186, 40, 40],
                    [92, 116, 72],
                    [176, 128, 44],
                    [56, 80, 128],
                    [142, 60, 96],
                    [72, 116, 120],
                    [130, 120, 106],
                ],
                font_family: "Courier New".into(),
                font_size: 17.0,
                font_weight: 700,
                line_height: 24.0,
                controls: Controls { style: ControlStyle::Brush, colors: [[158, 28, 32], [70, 64, 58], [70, 64, 58]] },
                //       bleed variation time nib grain edge absorb texture stains splatter
                params: vec![0.7, 0.35, 2.5, 1.2, 0.22, 0.7, 0.7, 1.0, 1.0, 1.0],
            },
            Variant {
                theme: self,
                name: "Tsushima Dark",
                dark: true,
                surface: [0.024, 0.019, 0.016],
                ink: [222, 196, 148],
                cursor: [212, 82, 54],
                selection: ([222, 196, 148], 0.88),
                palette: [
                    [104, 90, 72],   // black: thin parchment, still visible on ink
                    [206, 72, 58],   // red: crimson seal
                    [154, 164, 104], // green: dry moss
                    [226, 182, 104], // yellow: gold leaf
                    [140, 152, 174], // blue: river mist
                    [190, 120, 124], // magenta: dried petal
                    [132, 168, 150], // cyan: weathered bronze
                    [214, 192, 150], // white: parchment
                    [132, 116, 94],
                    [228, 98, 80],
                    [178, 188, 126],
                    [240, 204, 132],
                    [168, 180, 200],
                    [214, 148, 150],
                    [160, 194, 176],
                    [240, 222, 184],
                ],
                font_family: "Courier New".into(),
                font_size: 17.0,
                font_weight: 700,
                line_height: 24.0,
                controls: Controls { style: ControlStyle::Brush, colors: [[196, 44, 46], [150, 144, 134], [150, 144, 134]] },
                params: vec![0.7, 0.35, 2.5, 1.2, 0.25, 0.6, 0.7, 1.0, 1.0, 1.0],
            },
        ]
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn shaders(&self) -> Shaders {
        Shaders {
            common: include_str!("common.wgsl"),
            surface: Some(include_str!("surface.wgsl")),
            ink: Some(include_str!("ink.wgsl")),
            shapes: Some(include_str!("shapes.wgsl")),
            transition: Some(include_str!("transition.wgsl")),
        }
    }

    fn data(&self, v: &Variant, ctx: &Ctx) -> Vec<u8> {
        let (shade, splatter) = paper(v);
        let [r, g, b] = splatter.map(|c| c as f32 / 255.0);
        theme::bytes(&Data {
            shade: [shade[0], shade[1], shade[2], v.param("texture")],
            splatter: [r, g, b, v.param("splatter")],
            stains: v.param("stains"),
            bleed: v.param("bleed") * ctx.scale,
            grain: v.param("grain"),
            edge: v.param("edge"),
            nib: v.param("nib") * ctx.scale,
            bleed_variation: v.param("bleed_variation"),
            bleed_time: v.param("bleed_time") / ctx.speed,
            absorb: v.param("absorb"),
        })
    }

    /// Brushwork made up for this window: every opening is different.
    fn reveal(&self, _v: &Variant, rng: &mut Rng, aspect: f32) -> Option<Plan> {
        let brush = Brush::random(rng, aspect);
        Some(Plan { duration: REVEAL_TIME, ease: brush.ease, data: theme::bytes(&PlanData::new(&brush, [0; 3])) })
    }

    /// A flurry of sword cuts made up at random, edged in the new look's
    /// seal ink, at a slightly varied pace.
    fn switch(&self, v: &Variant, rng: &mut Rng, aspect: f32) -> Option<Plan> {
        let brush = Brush::sword(rng, aspect);
        let pace = rng.range(0.85, 1.2);
        Some(Plan { duration: SWITCH_TIME * pace, ease: brush.ease, data: theme::bytes(&PlanData::new(&brush, v.cursor)) })
    }

    /// The bleed keeps creeping for `bleed_time`.
    fn settle_time(&self, v: &Variant) -> f32 {
        v.param("bleed_time")
    }

    /// The cursor is a dab of seal ink in the ink layer, so it bleeds and
    /// takes the paper's grain like the text. It goes under the text, so a
    /// block cursor shows its character in the paper's colour.
    fn cursor(&self, v: &Variant, _ctx: &Ctx, cursor: &Cursor, out: &mut Vec<Shape>) {
        let [x, y, cw, ch] = cursor.cell;
        let c = v.cursor;
        let seed = cursor.column as f32 * 1.37 + cursor.line as f32 * 7.11;
        let t = (cw * 0.1).max(1.0);
        match cursor.kind {
            CursorKind::Beam => out.push(dab(x, y, (cw * 0.18).max(2.5), ch, c, 1.0).rough(0.6, seed)),
            CursorKind::Underline => out.push(dab(x, y + ch * 0.84, cw, (ch * 0.12).max(2.5), c, 1.0).rough(0.6, seed)),
            CursorKind::Hollow => out.extend([
                Shape::new(x, y, cw, t, c, 0.9),
                Shape::new(x, y + ch - t, cw, t, c, 0.9),
                Shape::new(x, y, t, ch, c, 0.9),
                Shape::new(x + cw - t, y, t, ch, c, 0.9),
            ]),
            CursorKind::Block => out.push(dab(x, y + ch * 0.04, cw, ch * 0.92, c, 0.95).rough(1.0, seed)),
        }
    }

    fn divider_time(&self) -> f32 {
        CUT_TIME
    }

    /// Panes are divided with one stroke of the sword: a long cut in sumi
    /// from end to end, with the hairline its edge scratched beside it.
    fn draw_divider(&self, v: &Variant, _ctx: &Ctx, span: &DividerSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.rect;
        let mut cut = Shape::new(x, y, w, h, v.ink, 0.8);
        cut.params = [CUT, span.progress, if span.upright { 1.0 } else { 0.0 }, span.seed];
        out.push(cut);
    }

    fn strike(&self) -> Option<Strike> {
        Some(Strike { duration: STRIKE_TIME, linger: SPLATTER_DRY, fade: STRIKE_FADE })
    }

    fn mark_time(&self) -> f32 {
        FAILURE_TIME
    }

    fn stamp_time(&self) -> f32 {
        STAMP_TIME
    }

    /// A long command is sealed when it finishes: a hanko in seal ink comes
    /// down after it, lands with a thud that jolts it a little, and leaves
    /// its carved strokes in the paper. A failed one is an empty seal with a
    /// slash through it.
    fn draw_stamp(&self, v: &Variant, _ctx: &Ctx, span: &CommandSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.after;
        let falling = span.progress * span.progress;
        let size = h * 1.2 * (1.0 + 0.45 * (1.0 - falling));
        let thud = ((span.age - STAMP_TIME) / THUD_TIME).clamp(0.0, 1.0);
        let shake = if span.progress >= 1.0 && thud < 1.0 { (thud * 40.0).sin() * (1.0 - thud) * h * 0.06 } else { 0.0 };
        let (cx, cy) = (x + w + h * 0.6 + shake, y + h * 0.5);
        let alpha = if span.progress < 1.0 { 0.25 + 0.5 * falling } else { 0.92 };
        let mut seal = Shape::new(cx - size / 2.0, cy - size / 2.0, size, size, v.cursor, alpha);
        seal.params = [SEAL, span.progress, if span.failed { 1.0 } else { 0.0 }, span.seed];
        out.push(seal);
    }

    /// A failed command gets a stroke of seal ink down the margin beside its
    /// prompt, pressed on at the top and dragged down, and a drop of it
    /// that runs a little further and dries.
    fn draw_failure(&self, v: &Variant, ctx: &Ctx, span: &CommandSpan, out: &mut Vec<Shape>) {
        let [mx, y, mw, h] = span.margin;
        let w = (h * 0.2).clamp(2.5 * ctx.scale, mw * 0.4);
        let x = mx + mw * 0.55 - w / 2.0;
        let len = h * 0.9 * span.progress;
        out.push(dab(x, y + h * 0.04, w, len, v.cursor, 0.95).rough(0.9, span.seed));
        // The drop gathers at the end of the stroke, then runs and dries.
        let run = ((span.age - FAILURE_TIME) / 0.9).clamp(0.0, 1.0);
        if span.progress >= 1.0 && run > 0.0 {
            let eased = 1.0 - (1.0 - run) * (1.0 - run);
            let r = w * 0.42;
            let cy = y + h * 0.94 + eased * h * 0.35;
            let alpha = 0.9 - 0.35 * ((span.age - FAILURE_TIME - 0.9) / 1.2).clamp(0.0, 1.0);
            out.push(dab(x + w * 0.5 - r * 0.45, y + h * 0.9, r * 0.9, cy - y - h * 0.9, v.cursor, alpha).rough(0.6, span.seed + 3.0));
            out.push(Shape::round(DISC, x + w * 0.5, cy, r, v.cursor, alpha).rough(0.3, span.seed + 5.0));
        }
    }

    /// Deleted names are cut through in seal ink, a little past each end,
    /// flinging blood off the blade.
    fn draw_strike(&self, v: &Variant, ctx: &Ctx, span: &StrikeSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.word;
        let cw = ctx.cell[0];
        let mut cut = Shape::new(x - cw * 0.3, y, w + cw * 0.6, h, v.cursor, 1.0);
        cut.params = [SLASH, span.progress, 0.0, span.seed];
        out.push(cut);
        if span.extras {
            splatter(out, span.word, span.age, span.seed, v.cursor);
        }
    }
}

/// A dab of the brush filling the box (x, y, w, h), with a pixel of room
/// around it for the ragged edge.
fn dab(x: f32, y: f32, w: f32, h: f32, rgb: [u8; 3], alpha: f32) -> Shape {
    Shape::new(x - 1.0, y - 1.0, w + 2.0, h + 2.0, rgb, alpha).kind(DAB)
}

/// Blood flung off a sword cut through a name in the box (x, y, w, h), `age`
/// seconds (at speed 1) after the cut began: drops leave the blade as it
/// passes them and flick a short way, mostly the way it moves, then splat
/// and dry to a faint stain. Everything stays inside the struck name's own
/// box, so no blood lands on the text around it. Big drops fall close; small
/// ones flick further, some with a fine mist.
fn splatter(out: &mut Vec<Shape>, [x, y, w, h]: [f32; 4], age: f32, seed: f32, rgb: [u8; 3]) {
    let mut rng = Rng::new(seed.to_bits() as u64 * 0x9e37_79b9 + 7);
    let drops = 3 + (w / h * 1.2) as usize;
    // Where drops may be: the line the name is on, end to end.
    let (top, bottom) = (y + h * 0.12, y + h * 0.95);
    let inside = |cx: f32, cy: f32, r: f32| (cx.clamp(x + r, x + w - r), cy.clamp(top + r, bottom - r));
    for _ in 0..drops.min(10) {
        let along = rng.range(0.1, 0.92);
        let release = along * STRIKE_TIME;
        let flight = rng.range(0.1, 0.18);
        let angle = rng.range(-0.9, 0.9);
        let reach = rng.unit();
        let distance = h * mix(0.15, 0.9, reach);
        let radius = h * mix(0.13, 0.05, reach) * rng.range(0.75, 1.15);
        let mist = rng.unit() < 0.35;
        let mist_seed = rng.unit();
        if age < release {
            continue;
        }
        let u = ((age - release) / flight).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - u) * (1.0 - u);
        let origin = (x + w * along, y + h * 0.55);
        // In flight a drop is smaller; it spreads as it lands.
        let r = (radius * if u < 1.0 { 0.65 } else { 1.0 }).max(0.8);
        let (cx, cy) = inside(origin.0 + angle.cos() * distance * eased, origin.1 + angle.sin() * distance * eased, r);
        let dried = ((age - release - flight - 0.5) / (SPLATTER_DRY - 0.5)).clamp(0.0, 1.0);
        let alpha = 0.95 - (0.95 - SPLATTER_STAIN) * dried;
        out.push(Shape::round(DISC, cx, cy, r, rgb, alpha).rough(0.35, seed + along * 17.0));
        if mist && u >= 1.0 {
            let fine = (h * 0.025).max(0.6);
            for k in 0..3 {
                let a = (mist_seed + k as f32 * 0.37) * std::f32::consts::TAU;
                let d = radius * (1.6 + k as f32 * 0.5);
                let (mx, my) = inside(cx + a.cos() * d, cy + a.sin() * d, fine);
                out.push(Shape::round(DISC, mx, my, fine, rgb, alpha * 0.8));
            }
        }
    }
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
