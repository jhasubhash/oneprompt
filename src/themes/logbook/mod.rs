//! Logbook: an old ship's log. Iron-gall ink on cream ledger paper, ruled
//! under every line of text with a red double margin, spotted with foxing,
//! a compass rose faint in one corner. The dark variant is the same log read
//! below deck by lamplight: a brown page, the lamp's warm pool in the middle,
//! the ink catching its glow.
//!
//! Iron-gall ink goes on blue-black and browns as it dries, so fresh text is
//! darker and bluer than old. The window opens and switches with a page
//! turning; deleted names are lined through with the quill, which leaves a
//! blot; a failed command gets a cross in the margin, a long one a wax seal.

use crate::theme::{
    self, CommandSpan, ControlStyle, Controls, Ctx, DividerSpan, Param, Plan, Rng, Shape, Shaders, Sound, SoundEvent,
    Strike, StrikeSpan, Theme, Variant, CUSTOM, DISC,
};

pub static LOGBOOK: Logbook = Logbook;

pub struct Logbook;

/// Shape kinds drawn by shapes.wgsl.
const QUILL: f32 = CUSTOM;
const BLOT: f32 = CUSTOM + 1.0;
const RULE: f32 = CUSTOM + 2.0;
const SEAL: f32 = CUSTOM + 3.0;

/// A page turning, at speed 1: opening, switching, closing, clearing.
const OPEN_TIME: f32 = 1.2;
const TURN_TIME: f32 = 1.4;
const CLOSE_TIME: f32 = 1.1;
const CLEAR_TIME: f32 = 0.8;
/// The quill line through a deleted name, and how long its blot spreads.
const STRIKE_TIME: f32 = 0.3;
const BLOT_TIME: f32 = 1.2;
/// A new divider is ruled in this fast; a failure's cross and a seal land.
const RULE_TIME: f32 = 0.5;
const CROSS_TIME: f32 = 0.35;
const SEAL_TIME: f32 = 0.3;

const PARAMS: &[Param] = &[
    Param { key: "rules", about: "the ruled lines under the text and the red margin, 0..1", min: 0.0, max: 1.0 },
    Param { key: "foxing", about: "brown age spots on the paper: 1 built-in, 0 none, up to 3", min: 0.0, max: 3.0 },
    Param { key: "feather", about: "how much the ink feathers into the paper's fibers, 0..1", min: 0.0, max: 1.0 },
    Param { key: "rose", about: "the compass rose in the corner: 1 built-in, 0 none, up to 2", min: 0.0, max: 2.0 },
];

/// What common.wgsl reads (its `Logbook` struct).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Data {
    rules: f32,
    foxing: f32,
    feather: f32,
    rose: f32,
    wear: f32,
    night: f32,
    _pad: [f32; 2],
}

/// What transition.wgsl reads (its `Plan` struct).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PlanData {
    /// The way the page turns: 1 right to left (forward), -1 back.
    way: f32,
    /// Tilt of the fold, radians.
    tilt: f32,
    seed: f32,
    _pad: f32,
}

impl Theme for Logbook {
    fn id(&self) -> &'static str {
        "logbook"
    }

    fn name(&self) -> &'static str {
        "Logbook"
    }

    fn about(&self) -> &'static str {
        "Iron-gall ink in an old ship's log: ruled ledger paper, a red margin, foxing; by lamplight in the dark."
    }

    fn variants(&'static self) -> Vec<Variant> {
        vec![
            Variant {
                theme: self,
                name: "Logbook Day",
                dark: false,
                surface: [0.925, 0.886, 0.792],
                ink: [44, 32, 26],
                cursor: [150, 42, 34],
                selection: ([58, 44, 36], 0.85),
                palette: [
                    [40, 30, 24],   // black: iron gall
                    [150, 42, 34],  // red: the margin's red ink
                    [70, 94, 60],   // green: verdigris
                    [148, 104, 40], // yellow: ochre
                    [38, 62, 104],  // blue: Prussian blue
                    [110, 50, 76],  // magenta: madder
                    [44, 96, 100],  // cyan: sea green
                    [118, 104, 88], // "white": faded ink
                    [92, 80, 68],
                    [176, 58, 44],
                    [88, 116, 74],
                    [174, 126, 50],
                    [54, 84, 136],
                    [136, 66, 98],
                    [58, 120, 124],
                    [146, 132, 112],
                ],
                font_family: "Courier New".into(),
                font_size: 17.0,
                font_weight: 400,
                line_height: 26.0,
                controls: Controls { style: ControlStyle::Ring, colors: [[150, 42, 34], [92, 80, 68], [92, 80, 68]] },
                //       rules foxing feather rose
                params: vec![0.7, 1.0, 0.5, 1.0],
            },
            Variant {
                theme: self,
                name: "Logbook Night",
                dark: true,
                surface: [0.118, 0.086, 0.063],
                ink: [234, 210, 166],
                cursor: [232, 150, 72],
                selection: ([234, 210, 166], 0.8),
                palette: [
                    [120, 98, 76],   // black: faded ink, still visible
                    [226, 112, 84],  // red
                    [168, 184, 120], // green
                    [236, 190, 106], // yellow: lamplight
                    [140, 162, 196], // blue
                    [204, 140, 150], // magenta
                    [138, 186, 174], // cyan
                    [226, 204, 162], // white: parchment
                    [150, 126, 100],
                    [240, 140, 106],
                    [190, 206, 146],
                    [246, 208, 136],
                    [170, 190, 220],
                    [222, 168, 176],
                    [168, 208, 196],
                    [246, 230, 196],
                ],
                font_family: "Courier New".into(),
                font_size: 17.0,
                font_weight: 400,
                line_height: 26.0,
                controls: Controls { style: ControlStyle::Ring, colors: [[226, 112, 84], [150, 126, 100], [150, 126, 100]] },
                params: vec![0.45, 0.6, 0.45, 0.8],
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
            rules: v.param("rules"),
            foxing: v.param("foxing"),
            feather: v.param("feather"),
            rose: v.param("rose"),
            wear: ctx.wear,
            night: if v.dark { 1.0 } else { 0.0 },
            _pad: [0.0; 2],
        })
    }

    /// The log is opened: its cover turns away to show the page.
    fn reveal(&self, _v: &Variant, rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        Some(turn(rng, 1.0, OPEN_TIME))
    }

    /// A page is turned to the new look, forward or back.
    fn switch(&self, _v: &Variant, rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        let way = if rng.unit() < 0.7 { 1.0 } else { -1.0 };
        Some(turn(rng, way, TURN_TIME))
    }

    /// A closed tab's page is turned over, the next one beneath; the window
    /// closing turns to the inside of the back cover.
    fn close(&self, _v: &Variant, rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        Some(turn(rng, 1.0, CLOSE_TIME))
    }

    /// Clearing turns to a fresh page.
    fn clear(&self, _v: &Variant, rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        Some(turn(rng, 1.0, CLEAR_TIME))
    }

    /// Fresh iron-gall ink stays dark and blue while it is wet.
    fn settle_time(&self, _v: &Variant) -> f32 {
        1.5
    }

    fn divider_time(&self) -> f32 {
        RULE_TIME
    }

    /// Panes are divided with a double line ruled in red ink, like the
    /// margin.
    fn draw_divider(&self, v: &Variant, _ctx: &Ctx, span: &DividerSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.rect;
        let mut rule = Shape::new(x, y, w, h, v.palette[1], 0.75);
        rule.params = [RULE, span.progress, if span.upright { 1.0 } else { 0.0 }, span.seed];
        out.push(rule);
    }

    fn strike(&self) -> Option<Strike> {
        Some(Strike { duration: STRIKE_TIME, linger: BLOT_TIME, fade: 0.4 })
    }

    /// A deleted name is lined through with the quill, a little past each
    /// end; as extras, the nib catches at the start and leaves a blot that
    /// spreads into the paper.
    fn draw_strike(&self, v: &Variant, ctx: &Ctx, span: &StrikeSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.word;
        let cw = ctx.cell[0];
        let mut line = Shape::new(x - cw * 0.25, y, w + cw * 0.5, h, v.ink, 0.95);
        line.params = [QUILL, span.progress, 0.0, span.seed];
        out.push(line);
        if span.extras {
            let spread = (span.age / BLOT_TIME).clamp(0.0, 1.0).sqrt();
            let r = h * (0.12 + 0.14 * spread);
            let mut blot = Shape::round(DISC, x - cw * 0.15, y + h * 0.55, r * 1.6, v.ink, 0.9);
            blot.params = [BLOT, spread, 0.0, span.seed];
            out.push(blot);
        }
    }

    fn mark_time(&self) -> f32 {
        CROSS_TIME
    }

    /// A failed command gets a cross in red ink in the margin, two quick
    /// strokes of the quill.
    fn draw_failure(&self, v: &Variant, _ctx: &Ctx, span: &CommandSpan, out: &mut Vec<Shape>) {
        let [mx, y, mw, h] = span.margin;
        let size = (h * 0.6).min(mw * 0.7);
        let (x, top) = (mx + (mw - size) / 2.0, y + (h - size) / 2.0);
        for (i, up) in [(0.0, 0.0), (1.0, 1.0)] {
            let progress = (span.progress * 2.0 - i).clamp(0.0, 1.0);
            if progress > 0.0 {
                let mut stroke = Shape::new(x, top, size, size, v.palette[1], 0.95);
                stroke.params = [QUILL, progress, 1.0 + up, span.seed + i * 2.3];
                out.push(stroke);
            }
        }
    }

    fn stamp_time(&self) -> f32 {
        SEAL_TIME
    }

    /// A long command is sealed in wax after it: a drop of red wax pressed
    /// with the ship's seal, an anchor (or, if it failed, a broken seal).
    fn draw_stamp(&self, v: &Variant, _ctx: &Ctx, span: &CommandSpan, out: &mut Vec<Shape>) {
        let [x, y, w, h] = span.after;
        let pressed = span.progress * span.progress;
        let size = h * 1.25 * (1.0 + 0.35 * (1.0 - pressed));
        let (cx, cy) = (x + w + h * 0.7, y + h * 0.5);
        let wax = if v.dark { [176, 54, 40] } else { [140, 30, 28] };
        let mut seal = Shape::new(cx - size / 2.0, cy - size / 2.0, size, size, wax, 0.35 + 0.6 * pressed);
        seal.params = [SEAL, span.progress, if span.failed { 1.0 } else { 0.0 }, span.seed];
        out.push(seal);
    }

    /// The quill scratching as you type and as it marks, the seal pressed
    /// down, pages turning (the eraser's soft rub).
    fn sound(&self, event: SoundEvent) -> Option<Sound> {
        Some(match event {
            SoundEvent::Key | SoundEvent::Strike | SoundEvent::Failure => Sound::Brush,
            SoundEvent::Stamp => Sound::Thud,
            SoundEvent::Clear | SoundEvent::Close => Sound::Eraser,
        })
    }
}

/// A page turning `way` (1: right to left), taking `duration`.
fn turn(rng: &mut Rng, way: f32, duration: f32) -> Plan {
    let data = PlanData { way, tilt: rng.range(-0.18, 0.18), seed: rng.unit() * 100.0, _pad: 0.0 };
    Plan { duration, ease: 1.6, data: theme::bytes(&data) }
}
