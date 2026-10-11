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

// The folds worn into the sheet (`look.wear`): a long one across and one
// down, at places of the sheet's own, then smaller creases, each appearing
// as the wear passes its own point. The surface draws them; the ink, lying
// on the same sheet, is rubbed off along them.
struct Folds {
    relief: f32, // > 0 on a ridge catching the light, < 0 in a valley's shadow
    rub: f32,    // how worn the sheet is along a fold here, 0..1
};

fn folds(uv: vec2<f32>) -> Folds {
    var out = Folds(0.0, 0.0);
    let wear = look.wear;
    if (wear <= 0.0) {
        return out;
    }
    let aspect = vec2<f32>(frame.resolution.x / frame.resolution.y, 1.0);
    let s = vec2<f32>(fract(frame.seed * 0.371), fract(frame.seed * 0.613));
    for (var k = 0; k < 6; k++) {
        let fk = f32(k);
        let due = select(0.15 + fk * 0.13, fk * 0.25, k < 2);
        let strength = smoothstep(due, due + 0.2, wear);
        if (strength <= 0.0) {
            continue;
        }
        let origin = vec2<f32>(fract(s.x + fk * 0.618), fract(s.y + fk * 0.382));
        let angle = select(hash(vec2<f32>(fk, 3.0)) * 3.1416, select(1.5708, 0.0, k == 0) + (s.x - 0.5) * 0.3, k < 2);
        let dir = vec2<f32>(cos(angle), sin(angle));
        let q = (uv - origin) * aspect;
        let along = dot(q, dir);
        let reach = select(0.35 + 0.3 * hash(vec2<f32>(fk, 9.0)), 2.0, k < 2);
        let wobble = (noise(vec2<f32>(along * 6.0, fk * 7.0)) - 0.5) * 0.02;
        let across = dot(q, vec2<f32>(-dir.y, dir.x)) + wobble;
        let fade = 1.0 - smoothstep(reach * 0.6, reach, abs(along));
        let weight = strength * fade * select(1.0, 0.55, k >= 2);
        // A valley in shadow on one side, a ridge catching the light on the other.
        let w = 0.0035;
        let valley = exp(-pow((across + w) / w, 2.0));
        let ridge = exp(-pow((across - w) / w, 2.0));
        out.relief += (ridge * 0.7 - valley) * weight;
        // Handling rubs the sheet along the fold a little either side of
        // it, unevenly: worse where the fold was pressed hardest.
        let patchy = 0.55 + 0.45 * noise(vec2<f32>(along * 40.0, fk * 3.0));
        out.rub = max(out.rub, exp(-pow(across / (w * 2.2), 2.0)) * weight * patchy);
    }
    return out;
}
