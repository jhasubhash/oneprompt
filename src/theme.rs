//! Themes describe the whole look of the terminal: the surface the text sits on
//! and the ink it is written with. Keeping them as plain data means new looks
//! (paper, chalkboard, linen, ...) are added here without touching the renderer.

/// Which procedural texture the background shader draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    /// Plain fill, for people who still want a flat background.
    Flat = 0,
    /// Cotton paper: soft fibers, grain and a warm vignette.
    Paper = 1,
    /// Slate chalkboard: smudges and fine dust.
    Chalkboard = 2,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: &'static str,
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
    pub font_family: &'static str,
    pub font_size: f32,
    pub line_height: f32,
}

impl Theme {
    pub fn surface_rgb(&self) -> [u8; 3] {
        self.surface.map(|c| (c * 255.0).round() as u8)
    }
}

pub fn builtin() -> Vec<Theme> {
    vec![
        Theme {
            name: "Ink on Paper",
            texture: Texture::Paper,
            surface: [0.93, 0.89, 0.80],
            surface_shade: [0.72, 0.64, 0.50],
            texture_strength: 1.0,
            ink: [28, 38, 74],
            cursor: [28, 38, 74],
            selection: ([250, 214, 70], 0.55),
            palette: [
                [42, 38, 32],    // black: carbon ink
                [168, 50, 45],   // red ink
                [63, 107, 53],   // green
                [154, 106, 18],  // ochre
                [28, 47, 110],   // prussian blue
                [122, 46, 92],   // plum
                [31, 95, 102],   // teal
                [107, 99, 85],   // "white": faded pencil
                [74, 68, 58],
                [192, 57, 43],
                [79, 127, 63],
                [176, 125, 26],
                [43, 74, 154],
                [147, 64, 122],
                [42, 122, 128],
                [138, 128, 112],
            ],
            font_family: "Courier New",
            font_size: 17.0,
            line_height: 24.0,
        },
        Theme {
            name: "Chalkboard",
            texture: Texture::Chalkboard,
            surface: [0.11, 0.16, 0.14],
            surface_shade: [0.22, 0.28, 0.25],
            texture_strength: 1.0,
            ink: [236, 236, 228],
            cursor: [247, 224, 139],
            selection: ([247, 224, 139], 0.3),
            palette: [
                [74, 92, 85],
                [242, 139, 130],
                [181, 227, 161],
                [247, 224, 139],
                [158, 201, 240],
                [227, 166, 216],
                [163, 227, 220],
                [236, 236, 228],
                [110, 130, 122],
                [250, 170, 160],
                [205, 240, 190],
                [252, 236, 170],
                [190, 220, 248],
                [240, 195, 232],
                [195, 240, 235],
                [255, 255, 250],
            ],
            font_family: "PT Mono",
            font_size: 16.0,
            line_height: 24.0,
        },
        Theme {
            name: "Classic",
            texture: Texture::Flat,
            surface: [0.02, 0.02, 0.025],
            surface_shade: [0.0, 0.0, 0.0],
            texture_strength: 0.0,
            ink: [210, 210, 210],
            cursor: [210, 210, 210],
            selection: ([70, 90, 140], 0.7),
            palette: [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ],
            font_family: "Menlo",
            font_size: 14.0,
            line_height: 19.0,
        },
    ]
}
