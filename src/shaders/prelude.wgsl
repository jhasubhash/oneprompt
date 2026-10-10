// Prepended to every stage of every theme (see theme/mod.rs): the frame
// uniform, and the noise everything is made of.

struct Frame {
    resolution: vec2<f32>,  // physical pixels
    scale: f32,             // physical pixels per logical pixel
    seed: f32,              // the sheet: a new pattern every session / switch
    time: f32,              // seconds, the clock births are measured on
    speed: f32,             // animation speed, 1 = normal
    progress: f32,          // transition, eased 0..1; >= 1 means none
    _pad0: f32,
    from_color: vec4<f32>,  // rgb; w = 1 reveals over the colour, 0 over the last frame
    grid_origin: vec2<f32>, // top-left of the terminal grid, pixels
    cell: vec2<f32>,        // cell size, pixels
    grid_size: vec2<f32>,   // columns, rows
    slot_size: vec2<f32>,   // pen-order atlas slot, pixels (cell + 2 px margin)
    write_time: f32,        // seconds to write one character; 0 = off
    atlas_columns: f32,     // slots per row in the pen-order atlas
    _pad1: vec2<f32>,
    surface: vec4<f32>,     // the variant's surface colour (sRGB)
    ink: vec4<f32>,         // its text colour
    accent: vec4<f32>,      // its cursor colour
};

@group(0) @binding(0) var<uniform> frame: Frame;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), s.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), s.x),
        s.y,
    );
}

fn rotate(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

struct FullScreen {
    @builtin(position) pos: vec4<f32>,
};

// One triangle that covers the whole screen.
fn full_screen(i: u32) -> FullScreen {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FullScreen;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    return out;
}
