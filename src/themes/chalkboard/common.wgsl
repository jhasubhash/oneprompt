// Shared by all of Chalkboard's stages: its values (mod.rs, `Data`) and the
// board's tooth, which the surface shows and the chalk (or charcoal)
// catches on.

struct Chalkboard {
    grit: f32,      // how broken chalk strokes are, 0..1
    dust: f32,      // the haze of dust around strokes, 0..1
    smudges: f32,   // old eraser marks on the board, 0..3
    ghosts: f32,    // faint traces of old writing, 0..3
    settle: f32,    // seconds fresh chalk keeps shedding dust (at this speed)
    stone: f32,     // 1: the light variant's white stone, 0: slate
    _pad: vec2<f32>,
};

@group(0) @binding(1) var<uniform> look: Chalkboard;

// The board's pattern offset for this session's seed, in logical pixels.
fn board_offset() -> vec2<f32> {
    return vec2<f32>(fract(frame.seed * 0.173) * 600.0, fract(frame.seed * 0.389) * 600.0);
}

// Height of the slate's tooth: chalk sticks to the high spots.
fn tooth(p: vec2<f32>) -> f32 {
    return noise(p * 1.1) * 0.5 + noise(p * 2.6) * 0.3 + hash(floor(p * 1.5)) * 0.2;
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
