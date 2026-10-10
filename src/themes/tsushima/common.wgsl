// Shared by all of Tsushima's stages: its values (mod.rs, `Data`), and the
// paper's tooth and fibers, which the surface draws and the ink takes on at
// the same places.

struct Tsushima {
    shade: vec4<f32>,       // rgb the paper darkens towards; w: texture strength
    splatter: vec4<f32>,    // rgb of the ink flecks in the corner; w: amount
    stains: f32,            // water-stain strength
    bleed: f32,             // reach of the bleed in physical pixels
    grain: f32,             // dry-brush streaks, 0..1
    edge: f32,              // pigment pooling at stroke rims, 0..1
    nib: f32,               // brush nib length in physical pixels
    bleed_variation: f32,   // per-character bleed: (1 -/+ this) x bleed
    bleed_time: f32,        // seconds the bleed keeps creeping; 0 = at once
    absorb: f32,            // how much the ink takes on the paper's grain
    wear: f32,              // how worn the sheet is from use, 0..1
    _pad: vec3<f32>,
};

@group(0) @binding(1) var<uniform> look: Tsushima;

// A thin line wherever stretched noise crosses its midpoint: reads as one
// long kozo fiber lying in the sheet.
fn fiber(px: vec2<f32>, angle: f32, offset: f32) -> f32 {
    // Warp the domain so fibers curl instead of running dead straight.
    let warp = vec2<f32>(noise(px * 0.012 + offset), noise(px * 0.012 + offset + 5.0)) * 24.0;
    let q = rotate(px + warp, angle) * vec2<f32>(0.01, 0.12) + offset;
    let n = noise(q) * 0.75 + noise(q * 2.1) * 0.25;
    let sparse = smoothstep(0.62, 0.8, noise(q * vec2<f32>(0.6, 0.08) + 9.0));
    return (1.0 - smoothstep(0.0, 0.03, abs(n - 0.5))) * sparse;
}

// Height of the paper's tooth: coarse pulp lumps plus fine grit.
fn tooth(p: vec2<f32>) -> f32 {
    return noise(p * 0.35) * 0.55 + noise(p * 0.9) * 0.3 + noise(p * 2.2) * 0.15;
}

// The sheet's pattern offset for this session's seed, in logical pixels.
fn sheet_offset() -> vec2<f32> {
    return vec2<f32>(fract(frame.seed * 0.137) * 400.0, fract(frame.seed * 0.291) * 400.0);
}
