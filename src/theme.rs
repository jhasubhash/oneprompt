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
    /// Text colour as sRGB bytes.
    pub ink: [u8; 4],
    /// Font family name; falls back to the system monospace font if missing.
    pub font_family: &'static str,
    pub font_size: f32,
    pub line_height: f32,
}

pub fn builtin() -> Vec<Theme> {
    vec![
        Theme {
            name: "Ink on Paper",
            texture: Texture::Paper,
            surface: [0.93, 0.89, 0.80],
            surface_shade: [0.72, 0.64, 0.50],
            texture_strength: 1.0,
            ink: [28, 38, 74, 255],
            font_family: "American Typewriter",
            font_size: 20.0,
            line_height: 28.0,
        },
        Theme {
            name: "Chalkboard",
            texture: Texture::Chalkboard,
            surface: [0.11, 0.16, 0.14],
            surface_shade: [0.22, 0.28, 0.25],
            texture_strength: 1.0,
            ink: [236, 236, 228, 255],
            font_family: "Chalkboard",
            font_size: 20.0,
            line_height: 28.0,
        },
        Theme {
            name: "Classic",
            texture: Texture::Flat,
            surface: [0.02, 0.02, 0.025],
            surface_shade: [0.0, 0.0, 0.0],
            texture_strength: 0.0,
            ink: [210, 210, 210, 255],
            font_family: "Menlo",
            font_size: 16.0,
            line_height: 22.0,
        },
    ]
}
