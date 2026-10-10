// Shared by all of Logbook's stages: its values (mod.rs, `Data`) and the
// paper's fibers, which the surface shows and the ink feathers along.

struct Logbook {
    rules: f32,    // ruled lines and the margin, 0..1
    foxing: f32,   // age spots, 0..3
    feather: f32,  // ink feathering into the fibers, 0..1
    rose: f32,     // the compass rose, 0..2
    wear: f32,     // how worn the page is from use, 0..1
    night: f32,    // 1: read by lamplight
    _pad: vec2<f32>,
};

@group(0) @binding(1) var<uniform> look: Logbook;

// The page's pattern offset for this session's seed, in logical pixels.
fn page_offset() -> vec2<f32> {
    return vec2<f32>(fract(frame.seed * 0.217) * 500.0, fract(frame.seed * 0.453) * 500.0);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var k = 0; k < 5; k++) {
        v += amp * noise(q);
        q = q * 2.03 + vec2<f32>(17.0, 9.0);
        amp *= 0.5;
    }
    return v;
}

// The paper's fibers run mostly across the page (laid paper): this is
// their direction at `p`, for the ink to feather along.
fn fiber_dir(p: vec2<f32>) -> vec2<f32> {
    let a = (noise(p * 0.01) - 0.5) * 0.9;
    return vec2<f32>(cos(a), sin(a));
}
