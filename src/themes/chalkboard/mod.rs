//! Chalkboard: writing on a well-used board. The light variant is charcoal
//! on a slab of white stone, the dark one chalk on black slate. Strokes are
//! grainy where they skip the surface's tooth and dusty at their edges; the
//! surface carries old eraser smudges and ghosts of earlier lessons.
//!
//! The window is wiped clean with an eraser, back and forth, when it opens
//! and when switching to Chalkboard; deleted names get a quick chalk line
//! through them that knocks a little dust off.

use crate::theme::{
    self, CommandSpan, ControlStyle, Controls, Ctx, Cursor, CursorKind, DividerSpan, IdleSpan, Param, Plan, Rng, Shape,
    Entrance, PanelSpan, ProgressSpan, ProgressState, Shaders, Sound, SoundEvent, SplashSpan, Strike, StrikeSpan, Theme, Variant, Weather, WeatherSpan, CUSTOM, DISC,
    RING,
};

pub static CHALKBOARD: Chalkboard = Chalkboard;

pub struct Chalkboard;

/// Shape kinds drawn by shapes.wgsl.
const STICK: f32 = CUSTOM;
const LINE: f32 = CUSTOM + 1.0;
const RULE: f32 = CUSTOM + 2.0;
const STROKE: f32 = CUSTOM + 3.0;
/// A failed command's cross, both strokes, at speed 1.
const CROSS_TIME: f32 = 0.4;
/// A long command's circled tick, at speed 1.
const TICK_TIME: f32 = 0.55;
/// Motes of chalk dust that settle on an idle board, over this long.
const MOTES: usize = 70;
const SETTLING: f32 = 24.0;
/// A paste's puff of dust settles over this long.
const PUFF_TIME: f32 = 0.8;
/// A new divider between panes is ruled in this fast, at speed 1.
const RULE_TIME: f32 = 0.45;

/// How long the eraser takes, at speed 1.
const REVEAL_TIME: f32 = 1.3;
const SWITCH_TIME: f32 = 1.8;
const CLOSE_TIME: f32 = 1.1;
const FLOW_TIME: f32 = 1.4;
const SLIDE_TIME: f32 = 0.9;
/// Fresh chalk sheds dust this long.
const SETTLE_TIME: f32 = 1.0;
/// The chalk line through a deleted name, and the dust it knocks off.
const STRIKE_TIME: f32 = 0.25;
const DUST_TIME: f32 = 1.4;

const PARAMS: &[Param] = &[
    Param { key: "grit", about: "how broken chalk strokes are where they skip the board, 0..1", min: 0.0, max: 1.0 },
    Param { key: "dust", about: "the haze of chalk dust around strokes, 0..1", min: 0.0, max: 1.0 },
    Param { key: "smudges", about: "old eraser swirls and dust on the board: 1 built-in, 0 none, up to 3", min: 0.0, max: 3.0 },
    Param { key: "ghosts", about: "faint traces of old writing: 1 built-in, 0 none, up to 3", min: 0.0, max: 3.0 },
];

/// What common.wgsl reads (its `Chalkboard` struct).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Data {
    grit: f32,
    dust: f32,
    smudges: f32,
    ghosts: f32,
    settle: f32,
    stone: f32,
    _pad: [f32; 2],
}

/// What transition.wgsl reads (its `Plan` struct).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PlanData {
    passes: f32,
    angle: f32,
    seed: f32,
    upward: f32,
    /// 0: the eraser wipes the window; 1: the chalk is blown off `area`
    /// from `at`; 2: `area`'s board slides away (up if `upward`).
    mode: f32,
    at: [f32; 2],
    _pad: f32,
    area: [f32; 4],
}

impl Theme for Chalkboard {
    fn id(&self) -> &'static str {
        "chalkboard"
    }

    fn name(&self) -> &'static str {
        "Chalkboard"
    }

    fn about(&self) -> &'static str {
        "Charcoal on white stone in the light, chalk on black slate in the dark."
    }

    fn variants(&'static self) -> Vec<Variant> {
        vec![
            Variant {
                theme: self,
                name: "Chalkboard Stone",
                dark: false,
                surface: [0.905, 0.894, 0.866],
                ink: [26, 25, 24],
                cursor: [168, 72, 46],
                selection: ([34, 32, 30], 0.85),
                palette: [
                    [30, 29, 28],    // black: charcoal
                    [160, 56, 46],   // red: sanguine
                    [66, 104, 62],   // green: green pastel
                    [150, 110, 34],  // yellow: ochre
                    [46, 76, 128],   // blue
                    [122, 62, 118],  // magenta
                    [40, 108, 108],  // cyan
                    [112, 109, 104], // "white": thin charcoal
                    [86, 83, 79],
                    [184, 70, 56],
                    [82, 126, 76],
                    [176, 132, 44],
                    [62, 96, 156],
                    [146, 80, 140],
                    [54, 130, 128],
                    [140, 136, 130],
                ],
                font_family: "Menlo".into(),
                font_size: 16.0,
                font_weight: 400,
                line_height: 24.0,
                controls: Controls { style: ControlStyle::Ring, colors: [[168, 72, 46], [112, 109, 104], [112, 109, 104]] },
                //       grit dust smudges ghosts
                params: vec![0.4, 0.4, 1.0, 1.0],
            },
            Variant {
                theme: self,
                name: "Chalkboard Slate",
                dark: true,
                surface: [0.078, 0.086, 0.086],
                ink: [226, 228, 222],
                cursor: [240, 200, 110],
                selection: ([226, 228, 222], 0.8),
                palette: [
                    [92, 98, 96],    // black: grey chalk, still visible on slate
                    [236, 132, 132], // red: pink chalk
                    [160, 212, 142], // green
                    [240, 214, 120], // yellow
                    [142, 186, 230], // blue
                    [206, 158, 218], // magenta
                    [142, 210, 204], // cyan
                    [206, 208, 200], // white
                    [128, 134, 130],
                    [248, 160, 152],
                    [188, 228, 168],
                    [248, 230, 160],
                    [174, 208, 242],
                    [226, 188, 232],
                    [176, 228, 222],
                    [244, 244, 236],
                ],
                font_family: "Menlo".into(),
                font_size: 16.0,
                font_weight: 400,
                line_height: 24.0,
                controls: Controls { style: ControlStyle::Ring, colors: [[236, 132, 132], [240, 214, 120], [160, 212, 142]] },
                params: vec![0.55, 0.45, 1.0, 1.0],
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
        theme::bytes(&Data {
            grit: v.param("grit"),
            dust: v.param("dust"),
            // A board in use collects eraser marks and ghosts of old lessons.
            smudges: v.param("smudges") * (1.0 + 1.5 * ctx.wear),
            ghosts: v.param("ghosts") * (1.0 + 2.0 * ctx.wear),
            settle: SETTLE_TIME / ctx.speed,
            stone: if v.dark { 0.0 } else { 1.0 },
            _pad: [0.0; 2],
        })
    }

    fn reveal(&self, _v: &Variant, rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        Some(wipe(rng, 2.0, 3.0, REVEAL_TIME))
    }

    fn switch(&self, _v: &Variant, rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        Some(wipe(rng, 3.0, 5.0, SWITCH_TIME))
    }

    /// Clearing the board blows the chalk off it: the marks break up into
    /// dust and drift away, from where the cursor was outwards, the clean
    /// board beneath.
    fn clear(&self, _v: &Variant, rng: &mut Rng, _aspect: f32, area: [f32; 4], at: [f32; 2]) -> Option<Plan> {
        let data = PlanData { passes: 1.0, angle: 0.0, seed: rng.unit() * 100.0, upward: 0.0, mode: 1.0, at, _pad: 0.0, area };
        Some(Plan { duration: FLOW_TIME, ease: 1.0, data: theme::bytes(&data) })
    }

    /// A closed pane's board slides up or down out of the frame, the panes
    /// taking its place behind it.
    fn close_pane(&self, _v: &Variant, rng: &mut Rng, _aspect: f32, area: [f32; 4]) -> Option<Plan> {
        let upward = if rng.unit() < 0.5 { 1.0 } else { 0.0 };
        let data = PlanData { passes: 1.0, angle: 0.0, seed: rng.unit() * 100.0, upward, mode: 2.0, at: [0.5, 0.5], _pad: 0.0, area };
        Some(Plan { duration: SLIDE_TIME, ease: 1.0, data: theme::bytes(&data) })
    }

    /// A closed tab is wiped off the board with the eraser, the next one
    /// under it; closing the window wipes it clean.
    fn close(&self, _v: &Variant, rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        Some(wipe(rng, 2.0, 3.0, CLOSE_TIME))
    }

    fn settle_time(&self, _v: &Variant) -> f32 {
        SETTLE_TIME
    }

    /// The cursor is rubbed in with the side of the chalk.
    fn cursor(&self, v: &Variant, ctx: &Ctx, cursor: &Cursor, out: &mut Vec<Shape>) {
        let [x, y, cw, ch] = cursor.cell;
        let seed = cursor.column as f32 * 1.91 + cursor.line as f32 * 5.37;
        // The stick drags a puff of dust along a jump, settling as it goes.
        if let Some(([fx, fy, _, _], faded)) = cursor.trail {
            let mut rng = Rng::new(seed.to_bits() as u64 + 5);
            for _ in 0..14 {
                let t = rng.unit();
                let (tx, ty) = (fx + (x - fx) * t + cw * rng.unit(), fy + (y - fy) * t + ch * (0.3 + 0.6 * rng.unit()));
                let r = rng.range(0.6, 1.6) * ctx.scale;
                out.push(Shape::round(DISC, tx, ty + faded * ch * 0.4, r, v.ink, 0.5 * (1.0 - faded) * t));
            }
        }
        let stick = |x, y, w, h, alpha| {
            let mut s = Shape::new(x, y, w, h, v.cursor, alpha).kind(STICK);
            s.params[1] = 1.0;
            s.params[3] = seed;
            s
        };
        match cursor.kind {
            CursorKind::Block => out.push(stick(x, y + ch * 0.06, cw, ch * 0.88, 0.7 + 0.3 * cursor.pulse)),
            CursorKind::Beam => out.push(stick(x, y, (cw * 0.2).max(2.5), ch, 1.0)),
            CursorKind::Underline => out.push(stick(x, y + ch * 0.82, cw, (ch * 0.14).max(2.5), 1.0)),
            CursorKind::Hollow => theme::plain_cursor(v, ctx, cursor, out),
        }
    }

    fn divider_time(&self) -> f32 {
        RULE_TIME
    }

    /// Panes are divided by a line ruled in chalk, end to end.
    fn draw_divider(&self, v: &Variant, _ctx: &Ctx, span: &DividerSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.rect;
        let mut rule = Shape::new(x, y, w, h, v.ink, 0.7);
        rule.params = [RULE, span.progress, if span.upright { 1.0 } else { 0.0 }, span.seed];
        out.push(rule);
    }

    fn mark_time(&self) -> f32 {
        CROSS_TIME
    }

    /// A failed command gets a quick cross in chalk in the margin beside
    /// its prompt, one stroke and then the other, in the board's red.
    fn draw_failure(&self, v: &Variant, _ctx: &Ctx, span: &CommandSpan, out: &mut Vec<Shape>) {
        let [mx, y, mw, h] = span.margin;
        let size = (h * 0.62).min(mw * 0.8);
        let (x, top) = (mx + (mw - size) / 2.0, y + (h - size) / 2.0);
        for (i, flip) in [(0.0, 0.0), (1.0, 1.0)] {
            let progress = (span.progress * 2.0 - i).clamp(0.0, 1.0);
            if progress > 0.0 {
                let mut stroke = Shape::new(x, top, size, size, v.palette[1], 0.95).kind(STROKE);
                stroke.params[1] = progress;
                stroke.params[2] = flip;
                stroke.params[3] = span.seed + i * 3.1;
                out.push(stroke);
            }
        }
    }

    fn stamp_time(&self) -> f32 {
        TICK_TIME
    }

    /// Progress is a chalk line drawn along the title strip, in the board's
    /// red if the program failed; a short dash moving to and fro when it
    /// can't say how far along.
    fn draw_progress(&self, v: &Variant, _ctx: &Ctx, span: &ProgressSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.strip;
        let rgb = if span.state == ProgressState::Error { v.palette[1] } else { v.ink };
        let (from, len) = match span.state {
            ProgressState::Busy => ((((span.time * 0.7).sin() * 0.5 + 0.5) * w * 0.8), w * 0.2),
            _ => (0.0, w * span.share),
        };
        if len < 1.0 {
            return;
        }
        let mut line = Shape::new(x + from, y + h * 0.55, len, h * 0.45, rgb, 0.8 * span.fade).kind(LINE);
        line.params[1] = 1.0;
        line.params[3] = 11.0;
        out.push(line);
    }

    /// The settings panel slides in from the right on its rails, like a
    /// sliding board, heavily, settling as it stops.
    fn panel_entrance(&self) -> (Entrance, f32) {
        (Entrance::Slide, 0.5)
    }

    /// The board's wooden frame along its leading edge, with a chalk tray
    /// at the bottom, and the frame's shadow on what is behind.
    fn draw_panel_edge(&self, v: &Variant, ctx: &Ctx, span: &PanelSpan, out: &mut Vec<Shape>) {
        let [_, y, _, h] = span.panel;
        let s = ctx.scale;
        let wood = if v.dark { [92, 66, 44] } else { [150, 112, 74] };
        let (frame, x) = (9.0 * s, span.edge);
        out.push(Shape::new(x - frame - 10.0 * s, y, 10.0 * s, h, [0, 0, 0], 0.15));
        out.push(Shape::new(x - frame, y, frame, h, wood, 1.0));
        out.push(Shape::new(x - frame + 2.0 * s, y, 1.5 * s, h, [255, 240, 220], 0.18));
        out.push(Shape::new(x - frame - 4.0 * s, y + h - 16.0 * s, 26.0 * s, 8.0 * s, wood, 1.0));
    }

    /// Chalk tapping as you type and as it marks, the eraser for clearing
    /// and closing.
    fn sound(&self, event: SoundEvent) -> Option<Sound> {
        Some(match event {
            SoundEvent::Key | SoundEvent::Strike | SoundEvent::Failure | SoundEvent::Stamp => Sound::Chalk,
            SoundEvent::Clear | SoundEvent::Close => Sound::Eraser,
        })
    }

    fn splash_time(&self) -> f32 {
        PUFF_TIME
    }

    /// Pasted text lands in a puff of chalk dust that billows out along the
    /// line and settles.
    fn draw_splash(&self, v: &Variant, ctx: &Ctx, span: &SplashSpan, out: &mut Vec<Shape>) {
        let [x, y, cw, ch] = span.cell;
        let mut rng = Rng::new(span.seed.to_bits() as u64 + 77);
        let length = (span.reach.min(60.0) * cw).max(cw * 4.0);
        let t = (span.age / PUFF_TIME).clamp(0.0, 1.0);
        for _ in 0..40 {
            let along = rng.unit();
            let (dx, dy) = (rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
            let spread = ch * (0.3 + 1.2 * t) * rng.range(0.3, 1.0);
            let r = rng.range(0.8, 2.2) * ctx.scale * (1.0 + t);
            let (cx, cy) = (x + along * length + dx * spread, y + ch * 0.5 + dy * spread * 0.7 + t * ch * 0.3);
            out.push(Shape::round(DISC, cx, cy, r, v.ink, 0.45 * (1.0 - t)));
        }
    }

    /// Rain in chalk: quick slanted dashes down the board. Snow: soft chalk
    /// motes drifting and swaying.
    fn draw_weather(&self, v: &Variant, ctx: &Ctx, span: &WeatherSpan, out: &mut Vec<Shape>) {
        let s = ctx.scale;
        match span.kind {
            Weather::Rain => {
                for i in 0..(20.0 + span.amount * 140.0) as usize {
                    let (x, y, _, own) = theme::weather_particle(span, i, 0.9, 0.2);
                    let len = (18.0 + 26.0 * own) * s;
                    let mut dash = Shape::new(x, y, len * 0.2, len, v.ink, 0.3 + 0.2 * own).kind(STROKE);
                    dash.params[1] = 1.0;
                    dash.params[2] = 1.0;
                    dash.params[3] = own * 30.0;
                    out.push(dash);
                }
            }
            Weather::Snow => {
                for i in 0..(20.0 + span.amount * 130.0) as usize {
                    let (x, y, t, own) = theme::weather_particle(span, i, 10.0, 0.05);
                    let sway = (t * 13.0 + own * 7.0).sin() * 9.0 * s;
                    out.push(Shape::round(DISC, x + sway, y, (1.0 + 2.2 * own) * s, v.ink, 0.6).rough(0.4, own * 20.0));
                }
            }
        }
    }

    fn idle_time(&self) -> f32 {
        SETTLING + 14.0
    }

    /// Left alone, chalk dust drifts down the board and settles along its
    /// bottom edge, as on a chalk tray. A key or a click puffs it away.
    fn draw_idle(&self, v: &Variant, ctx: &Ctx, span: &IdleSpan, out: &mut Vec<Shape>) {
        let [ax, ay, aw, ah] = span.area;
        let s = ctx.scale;
        let mut rng = Rng::new(span.seed.to_bits() as u64 * 0x9e37_79b9 + 11);
        for i in 0..MOTES {
            let start = rng.range(0.0, SETTLING);
            let x0 = ax + rng.unit() * aw;
            let drift = rng.range(-40.0, 40.0) * s;
            let rest_y = ay + ah - rng.range(2.0, 9.0) * s;
            let fall = rng.range(8.0, 13.0);
            let r = rng.range(0.7, 2.1) * s;
            let wobble = rng.range(0.3, 0.8);
            let (puff_x, puff_y) = (rng.range(-1.0, 1.0), rng.range(0.3, 1.0));
            let t = span.elapsed - start;
            if t < 0.0 {
                continue;
            }
            let u = (t / fall).min(1.0);
            let mut x = x0 + drift * u + (t * wobble + i as f32).sin() * 6.0 * s * (1.0 - u);
            let mut y = mix(ay + ah * 0.05, rest_y, u);
            let mut alpha = 0.5 * (t / 1.5).min(1.0);
            if span.sweep > 0.0 {
                let puff = 1.0 - (1.0 - span.sweep) * (1.0 - span.sweep);
                x += puff_x * puff * 60.0 * s;
                y -= puff_y * puff * 50.0 * s;
                alpha *= 1.0 - span.sweep;
            }
            out.push(Shape::round(DISC, x, y, r, v.ink, alpha).rough(0.4, span.seed + i as f32));
        }
    }

    /// A long command gets a circle chalked after it when it finishes, then
    /// a tick in it (a cross if it failed), as a teacher marks work.
    fn draw_stamp(&self, v: &Variant, ctx: &Ctx, span: &CommandSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.after;
        let size = h * 1.05;
        let (left, top) = (x + w + h * 0.15, y + (h - size) / 2.0);
        let rgb = if span.failed { v.palette[1] } else { v.palette[2] };
        let ring = (span.progress * 2.0).clamp(0.0, 1.0);
        let r = size * 0.48;
        out.push(Shape::round(RING, left + size / 2.0, top + size / 2.0, r, rgb, 0.85 * ring).stroke(2.2 * ctx.scale).rough(0.12, span.seed));
        let mark = (span.progress * 2.0 - 1.0).clamp(0.0, 1.0);
        let stroke = |bx: f32, by: f32, bw: f32, bh: f32, up: bool, progress: f32, seed: f32| {
            let mut s = Shape::new(left + bx * size, top + by * size, bw * size, bh * size, rgb, 0.95).kind(STROKE);
            s.params[1] = progress;
            s.params[2] = if up { 1.0 } else { 0.0 };
            s.params[3] = seed;
            s
        };
        if mark > 0.0 {
            if span.failed {
                out.push(stroke(0.22, 0.22, 0.56, 0.56, false, (mark * 2.0).min(1.0), span.seed));
                if mark > 0.5 {
                    out.push(stroke(0.22, 0.22, 0.56, 0.56, true, mark * 2.0 - 1.0, span.seed + 1.0));
                }
            } else {
                out.push(stroke(0.18, 0.42, 0.3, 0.3, false, (mark * 3.0).min(1.0), span.seed));
                if mark > 0.33 {
                    out.push(stroke(0.36, 0.14, 0.5, 0.6, true, (mark * 1.5 - 0.5).min(1.0), span.seed + 1.0));
                }
            }
        }
    }

    fn strike(&self) -> Option<Strike> {
        Some(Strike { duration: STRIKE_TIME, linger: DUST_TIME, fade: 0.5 })
    }

    /// A quick chalk line through the name, a little past each end; as extras,
    /// a few motes of dust knocked off it drift down and fade.
    fn draw_strike(&self, v: &Variant, ctx: &Ctx, span: &StrikeSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.word;
        let cw = ctx.cell[0];
        let mut line = Shape::new(x - cw * 0.25, y, w + cw * 0.5, h, v.ink, 0.95).kind(LINE);
        line.params[1] = span.progress;
        line.params[3] = span.seed;
        out.push(line);
        if span.extras {
            dust(out, span.word, span.age, span.seed, v.ink);
        }
    }
}

/// An eraser wipe in `lo..=hi` passes, taking `duration`.
fn wipe(rng: &mut Rng, lo: f32, hi: f32, duration: f32) -> Plan {
    let data = PlanData {
        passes: rng.range(lo, hi + 0.99).floor(),
        angle: rng.range(-0.12, 0.12),
        seed: rng.unit() * 100.0,
        upward: if rng.unit() < 0.25 { 1.0 } else { 0.0 },
        mode: 0.0,
        at: [0.5, 0.5],
        _pad: 0.0,
        area: [0.0, 0.0, 1.0, 1.0],
    };
    Plan { duration, ease: 1.3, data: theme::bytes(&data) }
}

/// Motes of chalk dust knocked off a line through the box (x, y, w, h),
/// `age` seconds after it began: each leaves the line as the chalk passes,
/// drifts down within the name's own line and fades.
fn dust(out: &mut Vec<Shape>, [x, y, w, h]: [f32; 4], age: f32, seed: f32, rgb: [u8; 3]) {
    let mut rng = Rng::new(seed.to_bits() as u64 * 0x2545_f491 + 3);
    let motes = 4 + (w / h * 1.5) as usize;
    for _ in 0..motes.min(14) {
        let along = rng.range(0.05, 0.95);
        let release = along * STRIKE_TIME;
        let fall = h * rng.range(0.1, 0.4);
        let drift = h * rng.range(-0.25, 0.25);
        let radius = (h * rng.range(0.03, 0.07)).max(0.7);
        if age < release {
            continue;
        }
        let u = ((age - release) / (DUST_TIME - release)).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - u) * (1.0 - u);
        let cx = (x + w * along + drift * eased).clamp(x, x + w);
        let cy = (y + h * 0.55 + fall * eased).min(y + h * 0.95);
        out.push(Shape::round(DISC, cx, cy, radius, rgb, 0.7 * (1.0 - u)));
    }
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
