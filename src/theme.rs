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
    /// How strong the texture is, 0 = flat.
    pub texture_strength: f32,
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
        // Inspired by the ink-wash look of Ghost of Tsushima: rough washi with
        // water stains, bold sumi ink that bleeds a little, a crimson seal.
        Theme {
            name: "Tsushima Light",
            dark: false,
            texture: Texture::Washi,
            surface: [0.86, 0.81, 0.72],
            surface_shade: [0.52, 0.42, 0.30],
            texture_strength: 1.0,
            ink: [22, 20, 18],
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
            ink_style: Ink { bleed: 1.3, grain: 0.22, edge: 0.7, nib: 1.2 },
            splatter: ([168, 30, 36], 1.0),
            controls: Controls {
                style: ControlStyle::Brush,
                colors: [[158, 28, 32], [70, 64, 58], [70, 64, 58]],
            },
        },
        // The same island at night, after konshi: indigo-black dyed paper that
        // sutras were written on in gold and white pigment. Pale gofun ink, the
        // same crimson seal, stains that dry lighter than the sheet.
        Theme {
            name: "Tsushima Dark",
            dark: true,
            texture: Texture::Washi,
            surface: [0.085, 0.095, 0.14],
            surface_shade: [0.22, 0.225, 0.27],
            texture_strength: 1.0,
            ink: [232, 222, 198],
            cursor: [196, 44, 46],
            selection: ([196, 64, 60], 0.3),
            palette: [
                [96, 94, 100],   // black: wet slate, still visible on the sheet
                [210, 70, 64],   // red: crimson seal
                [140, 170, 116], // green: young moss
                [214, 172, 92],  // yellow: gold leaf
                [128, 156, 206], // blue: pale indigo
                [196, 122, 160], // magenta: plum blossom
                [116, 176, 176], // cyan: river teal
                [214, 206, 188], // white: gofun
                [130, 128, 134],
                [232, 96, 86],
                [166, 196, 140],
                [234, 196, 120],
                [160, 186, 230],
                [222, 150, 186],
                [146, 204, 202],
                [244, 238, 222],
            ],
            font_family: "Courier New".into(),
            font_size: 17.0,
            font_weight: 700,
            line_height: 24.0,
            ink_style: Ink { bleed: 1.1, grain: 0.25, edge: 0.6, nib: 1.2 },
            splatter: ([186, 40, 44], 1.0),
            controls: Controls {
                style: ControlStyle::Brush,
                colors: [[196, 44, 46], [150, 144, 134], [150, 144, 134]],
            },
        },
    ]
}
