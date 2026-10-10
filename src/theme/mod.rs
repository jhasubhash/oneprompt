//! The theme API: everything a theme may use, and the only part of the app
//! a theme may use. A theme (`src/themes/<name>/`) implements [`Theme`]: its
//! variants (light, dark, ...), its settings, the shaders for the stages it
//! draws itself, and how it reacts to events (the window opening, switching
//! to it, a deleted name being struck out). The framework does the rest:
//! the terminal, text layout, render passes, settings files, the picker.
//!
//! Themes never depend on each other; `themes::tests` checks that a theme's
//! code only reaches into the app through `crate::theme`.
//!
//! # Shaders
//!
//! Each stage is a WGSL function the framework calls from its own entry
//! points. Every stage gets `prelude.wgsl` (the `frame` uniform, `hash`,
//! `noise`, `rotate`), then the theme's `common` source, then the stage's own
//! framework part, then the theme's function:
//!
//! - surface: `fn surface(px: vec2<f32>, uv: vec2<f32>) -> vec3<f32>`, px in
//!   logical pixels: the sheet the text sits on.
//! - ink: `fn theme_ink(px: vec2<f32>) -> vec4<f32>`, physical pixels,
//!   premultiplied: how the ink layer (text, line art, controls) lands on the
//!   surface. `sample_at`, `written` and `cell_ink` (ink_stage.wgsl) give the
//!   layer and the hand-writing state.
//! - shapes: `fn theme_shape(s: ShapeIn) -> vec4<f32>` for the theme's own
//!   [`Shape`] kinds (from [`CUSTOM`] up).
//! - transition: `fn theme_transition(px: vec2<f32>, now: vec4<f32>, before:
//!   vec4<f32>) -> vec4<f32>`: the window revealed over what was there, while
//!   `frame.progress` < 1.
//!
//! A theme's values reach its shaders through `@group(0) @binding(1)` (the
//! bytes [`Theme::data`] returns, up to [`DATA_SIZE`]) and, in the transition,
//! `@group(0) @binding(2)` (the bytes of the running [`Plan`], up to
//! [`PLAN_SIZE`]), each declared by the theme with its own struct.

use std::fmt;

pub use crate::animation::Rng;

/// Most bytes [`Theme::data`] may return.
pub const DATA_SIZE: usize = 256;
/// Most bytes a [`Plan`] may carry.
pub const PLAN_SIZE: usize = 1024;

pub trait Theme: Sync + 'static {
    /// Stable identifier, used in settings: `theme = "tsushima"`.
    fn id(&self) -> &'static str;
    /// Shown in the picker.
    fn name(&self) -> &'static str;
    /// One sentence about the look, for the picker and the settings file.
    fn about(&self) -> &'static str;
    /// The theme's variants with their built-in values. Settings pick one by
    /// appearance: the first light one or the first dark one.
    fn variants(&'static self) -> Vec<Variant>;
    /// The theme's own settings; every variant holds a value for each, in
    /// this order (`Variant::params`).
    fn params(&self) -> &'static [Param] {
        &[]
    }
    fn shaders(&self) -> Shaders;
    /// The theme's values for its shaders, at most [`DATA_SIZE`] bytes.
    fn data(&self, _variant: &Variant, _ctx: &Ctx) -> Vec<u8> {
        Vec::new()
    }
    /// How the window is revealed when it opens. None: it simply appears.
    fn reveal(&self, _variant: &Variant, _rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        None
    }
    /// How this theme takes over the window, from another variant of itself
    /// or from another theme. None: it simply appears.
    fn switch(&self, _variant: &Variant, _rng: &mut Rng, _aspect: f32) -> Option<Plan> {
        None
    }
    /// Seconds the ink keeps changing after a character is written (at
    /// speed 1), so the window keeps redrawing that long.
    fn settle_time(&self, _variant: &Variant) -> f32 {
        0.0
    }
    /// Draws the cursor into the ink layer. The default is plain blocks and
    /// bars.
    fn cursor(&self, variant: &Variant, ctx: &Ctx, cursor: &Cursor, out: &mut Vec<Shape>) {
        plain_cursor(variant, ctx, cursor, out);
    }
    /// How names of deleted files are struck out; None: they are not.
    fn strike(&self) -> Option<Strike> {
        None
    }
    /// Draws one struck-out name (see [`Strike`]) into the ink layer.
    fn draw_strike(&self, _variant: &Variant, _ctx: &Ctx, _span: &StrikeSpan, _out: &mut Vec<Shape>) {}
}

impl fmt::Debug for dyn Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// One of a theme's settings (`[themes.<variant>]` in the settings file).
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub key: &'static str,
    /// What it does, for the settings file.
    pub about: &'static str,
    pub min: f32,
    pub max: f32,
}

/// WGSL sources for the stages a theme draws itself; the others use the
/// framework's plain defaults.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shaders {
    /// Prepended to each of the theme's stages: shared structs, bindings and
    /// helpers.
    pub common: &'static str,
    pub surface: Option<&'static str>,
    pub ink: Option<&'static str>,
    pub shapes: Option<&'static str>,
    pub transition: Option<&'static str>,
}

/// What theme hooks need to know about the window.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// Cell size, physical pixels.
    pub cell: [f32; 2],
    /// Animation speed: 1 normal, 2 twice as fast.
    pub speed: f32,
}

/// A reveal or a switch: how long it takes (at speed 1), how hard progress
/// eases out (1 runs evenly, 3 rushes in and lingers), and the bytes the
/// theme's transition shader reads (`@binding(2)`).
#[derive(Clone, Debug)]
pub struct Plan {
    pub duration: f32,
    pub ease: f32,
    pub data: Vec<u8>,
}

/// How a theme strikes out names: how long the strike takes (at speed 1),
/// how long it keeps moving after that (drying, settling), and how far the
/// struck text fades towards the surface, 0..1.
#[derive(Clone, Copy, Debug)]
pub struct Strike {
    pub duration: f32,
    pub linger: f32,
    pub fade: f32,
}

/// One struck-out name on screen.
#[derive(Clone, Copy, Debug)]
pub struct StrikeSpan {
    /// The name's cells (x, y, w, h), physical pixels.
    pub word: [f32; 4],
    /// How far the strike has got, 0..1.
    pub progress: f32,
    /// Seconds since it began, at speed 1 (infinite with animations off).
    pub age: f32,
    /// Stable for this name while it scrolls.
    pub seed: f32,
    /// Extras on top of the strike itself (`strike_splatter`).
    pub extras: bool,
}

/// Where the cursor is and what the terminal asks it to look like.
#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    /// Its cell (x, y, w, h), physical pixels.
    pub cell: [f32; 4],
    pub column: usize,
    pub line: usize,
    pub kind: CursorKind,
}

/// What the terminal asks the cursor to look like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Block,
    Beam,
    Underline,
    /// The window is in the background.
    Hollow,
}

/// One light or dark look of a theme: colours, fonts and the values of the
/// theme's settings.
#[derive(Clone)]
pub struct Variant {
    pub theme: &'static dyn Theme,
    pub name: &'static str,
    /// Light text on a dark surface: chosen when the system is in dark mode.
    pub dark: bool,
    /// Base colour of the surface (sRGB, 0..1).
    pub surface: [f32; 3],
    /// Default text colour (sRGB).
    pub ink: [u8; 3],
    /// Cursor and other marks of the hand (tab underline, caret).
    pub cursor: [u8; 3],
    /// Selection block colour and opacity; selected text is drawn on it in
    /// the surface colour.
    pub selection: ([u8; 3], f32),
    /// The 16 ANSI colours, re-imagined for this surface.
    pub palette: [[u8; 3]; 16],
    /// Must be monospace: the terminal grid relies on every cell having the
    /// same advance. Falls back to the system monospace font if missing.
    pub font_family: String,
    pub font_size: f32,
    /// CSS-style weight: 400 regular, 700 bold. Bold cells use at least 700.
    pub font_weight: u16,
    pub line_height: f32,
    pub controls: Controls,
    /// Values of the theme's settings, in `Theme::params` order.
    pub params: Vec<f32>,
}

impl fmt::Debug for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Variant").field("theme", &self.theme.id()).field("name", &self.name).finish()
    }
}

impl Variant {
    pub fn surface_rgb(&self) -> [u8; 3] {
        self.surface.map(|c| (c * 255.0).round() as u8)
    }

    /// The value of the theme's setting `key` (0 if it has none).
    pub fn param(&self, key: &str) -> f32 {
        self.theme.params().iter().position(|p| p.key == key).and_then(|i| self.params.get(i)).copied().unwrap_or(0.0)
    }

    /// Its section in the settings file: "Tsushima Dark" -> "tsushima-dark".
    pub fn key(&self) -> String {
        key(self.name)
    }
}

/// Names as they appear in settings: "Tsushima Dark" -> "tsushima-dark".
pub fn key(name: &str) -> String {
    name.trim().to_lowercase().replace([' ', '_'], "-")
}

/// How the window's close / minimize / zoom controls are drawn.
#[allow(dead_code)] // Dot is the default style; no theme uses it yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlStyle {
    /// Clean circles, like the system's own.
    Dot,
    /// Uneven brush-dabbed circles.
    Brush,
    /// Outlined circles, like chalk.
    Ring,
}

#[derive(Clone, Copy, Debug)]
pub struct Controls {
    pub style: ControlStyle,
    /// Close, minimize, zoom.
    pub colors: [[u8; 3]; 3],
}

/// Shape kinds the framework draws (rect.wgsl).
pub const FILL: f32 = 0.0;
pub const DISC: f32 = 1.0;
pub const RING: f32 = 2.0;
pub const CROSS: f32 = 3.0;
/// The first kind a theme draws itself, in its `theme_shape`.
pub const CUSTOM: f32 = 16.0;

/// A box drawn by rect.wgsl: a filled rectangle, a framework shape, or one
/// of the theme's own (`kind` from [`CUSTOM`] up).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Shape {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    /// Kind, then three values the shape interprets (for the framework's:
    /// roughness, stroke width, seed).
    pub params: [f32; 4],
}

impl Shape {
    pub fn new(x: f32, y: f32, w: f32, h: f32, rgb: [u8; 3], alpha: f32) -> Self {
        let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
        Self { rect: [x, y, w, h], color: [r, g, b, alpha], params: [FILL, 0.0, 0.0, 0.0] }
    }

    /// A round shape centred on (cx, cy).
    pub fn round(kind: f32, cx: f32, cy: f32, radius: f32, rgb: [u8; 3], alpha: f32) -> Self {
        let d = radius * 2.0 + 2.0;
        Self::new(cx - d / 2.0, cy - d / 2.0, d, d, rgb, alpha).kind(kind)
    }

    pub fn kind(mut self, kind: f32) -> Self {
        self.params[0] = kind;
        self
    }

    /// Edge roughness of a disc or ring (brush-like), and its seed.
    pub fn rough(mut self, roughness: f32, seed: f32) -> Self {
        self.params[1] = roughness;
        self.params[3] = seed;
        self
    }

    pub fn stroke(mut self, width: f32) -> Self {
        self.params[2] = width;
        self
    }
}

/// The plain cursor: a block, a bar or an underline in the cursor colour.
pub fn plain_cursor(variant: &Variant, ctx: &Ctx, cursor: &Cursor, out: &mut Vec<Shape>) {
    let [x, y, w, h] = cursor.cell;
    let c = variant.cursor;
    let t = (w * 0.1).max(1.0);
    match cursor.kind {
        CursorKind::Block => out.push(Shape::new(x, y, w, h, c, 1.0)),
        CursorKind::Beam => out.push(Shape::new(x, y, (1.5 * ctx.scale).max(1.0), h, c, 1.0)),
        CursorKind::Underline => out.push(Shape::new(x, y + h - t * 1.5, w, t * 1.5, c, 1.0)),
        CursorKind::Hollow => out.extend([
            Shape::new(x, y, w, t, c, 0.9),
            Shape::new(x, y + h - t, w, t, c, 0.9),
            Shape::new(x, y, t, h, c, 0.9),
            Shape::new(x + w - t, y, t, h, c, 0.9),
        ]),
    }
}

/// The bytes of a plain-old-data value, for [`Theme::data`] and [`Plan`].
pub fn bytes<T: bytemuck::Pod>(value: &T) -> Vec<u8> {
    bytemuck::bytes_of(value).to_vec()
}

/// Every theme, in picker order.
pub fn all() -> Vec<&'static dyn Theme> {
    crate::themes::all()
}

/// Every variant of every theme, with built-in values.
pub fn builtin() -> Vec<Variant> {
    all().into_iter().flat_map(|t| t.variants()).collect()
}
