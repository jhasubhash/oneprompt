//! Themes describe the whole look of the terminal: the surface the text sits on
//! and the ink it is written with. Keeping them as plain data means new looks
//! (paper, chalkboard, linen, ...) are added here without touching the renderer.
//!
//! For now there is one theme, Tsushima, in a light and a dark variant. The
//! renderer still supports the other surfaces and control styles below, for
//! themes that will come back later.

/// Which procedural texture the background shader draws.
#[allow(dead_code)] // Flat, Paper and Chalkboard are unused by the current themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    /// Plain fill, for people who still want a flat background.
    Flat = 0,
    /// Cotton paper: soft fibers, grain and a warm vignette.
    Paper = 1,
    /// Slate chalkboard: smudges and fine dust.
    Chalkboard = 2,
    /// Hand-made washi: pulp clouds, long fibers, water-stain tide lines and
    /// a few flicks of ink splatter (coloured by `Theme::splatter`).
    Washi = 3,
}

/// How ink sits on the surface. All zero means crisp screen text.
#[derive(Clone, Copy, Debug)]
pub struct Ink {
    /// How far ink wicks past a stroke, in logical pixels.
    pub bleed: f32,
    /// Dry-brush texture inside strokes, 0..1.
    pub grain: f32,
    /// Darkening where pigment pools at a stroke's rim, 0..1.
    pub edge: f32,
    /// Brush nib length in logical pixels: glyphs are swept along a slanted
    /// nib, giving thick-and-thin strokes. 0 keeps the font's own shapes.
    pub nib: f32,
    /// How much the bleed differs from character to character: each one
    /// bleeds between (1 - this) and (1 + this) times `bleed`. 0..1.
    pub bleed_variation: f32,
    /// Seconds the bleed keeps creeping outwards after a character is
    /// written (at animation speed 1), fast at first, then slowing.
    pub bleed_time: f32,
}

/// How the window's close / minimize / zoom controls are drawn.
#[allow(dead_code)] // Dot and Ring are unused by the current themes.
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

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: &'static str,
    /// Light text on a dark surface: chosen when the system is in dark mode.
    pub dark: bool,
    pub texture: Texture,
    /// Base colour of the surface (sRGB, 0..1).
    pub surface: [f32; 3],
    /// Darker tone the texture mixes towards in fibers, smudges and edges.
    pub surface_shade: [f32; 3],
    /// How strong the paper texture is (mottling, relief, fibers), 0 = flat.
    pub texture_strength: f32,
    /// How strong the water-stain tide lines are (washi), 0 = none.
    pub stains: f32,
    /// Default text colour (sRGB).
    pub ink: [u8; 3],
    pub cursor: [u8; 3],
    /// Selection colour and opacity, drawn under the text like a highlighter.
    pub selection: ([u8; 3], f32),
    /// The 16 ANSI colours, re-imagined for this surface. On paper "white"
    /// has to stay dark enough to read, so these are inks, not screen colours.
    pub palette: [[u8; 3]; 16],
    /// Must be monospace: the terminal grid relies on every cell having the
    /// same advance. Falls back to the system monospace font if missing.
    pub font_family: String,
    pub font_size: f32,
    /// CSS-style weight: 400 regular, 700 bold. Bold cells use at least 700.
    pub font_weight: u16,
    pub line_height: f32,
    pub ink_style: Ink,
    /// Colour and amount of the background's ink splatter (washi only).
    pub splatter: ([u8; 3], f32),
    pub controls: Controls,
}

impl Theme {
    pub fn surface_rgb(&self) -> [u8; 3] {
        self.surface.map(|c| (c * 255.0).round() as u8)
    }
}

pub fn builtin() -> Vec<Theme> {
    vec![
        // After the ink-painting cutscenes of Ghost of Tsushima / Yotei: golden
        // parchment darkening to brown at the edges, near-black sumi that
        // bleeds a little, a crimson seal.
        Theme {
            name: "Tsushima Light",
            dark: false,
            texture: Texture::Washi,
            surface: [0.886, 0.769, 0.557],
            surface_shade: [0.36, 0.26, 0.14],
            texture_strength: 1.0,
            stains: 1.0,
            ink: [18, 13, 9],
            cursor: [158, 28, 32],
            selection: ([196, 64, 60], 0.22),
            palette: [
                [22, 20, 18],    // black: sumi
                [158, 28, 32],   // red: crimson seal
                [72, 92, 58],    // green: moss
                [150, 108, 36],  // yellow: old gold
                [38, 56, 92],    // blue: indigo
                [116, 44, 74],   // magenta: plum
                [56, 92, 96],    // cyan: weathered teal
                [102, 92, 80],   // "white": thin ink wash
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
            ink_style: Ink { bleed: 1.3, grain: 0.22, edge: 0.7, nib: 1.2, bleed_variation: 0.35, bleed_time: 2.5 },
            splatter: ([168, 30, 36], 1.0),
            controls: Controls {
                style: ControlStyle::Brush,
                colors: [[158, 28, 32], [70, 64, 58], [70, 64, 58]],
            },
        },
        // The same scenes inverted, as the cutscenes do: a field of ink, with
        // the drawing showing through in parchment, and the crimson seal.
        Theme {
            name: "Tsushima Dark",
            dark: true,
            texture: Texture::Washi,
            surface: [0.024, 0.019, 0.016],
            surface_shade: [0.2, 0.158, 0.115],
            texture_strength: 1.0,
            stains: 1.0,
            ink: [222, 196, 148],
            cursor: [196, 44, 46],
            selection: ([196, 64, 60], 0.3),
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
            ink_style: Ink { bleed: 1.1, grain: 0.25, edge: 0.6, nib: 1.2, bleed_variation: 0.35, bleed_time: 2.5 },
            splatter: ([186, 40, 44], 1.0),
            controls: Controls {
                style: ControlStyle::Brush,
                colors: [[196, 44, 46], [150, 144, 134], [150, 144, 134]],
            },
        },
    ]
}
