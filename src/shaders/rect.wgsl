// Shapes in pixel space: cell backgrounds, cursor, underlines, box drawing
// and the window controls. Each instance is a box with a shape kind:
//   0 filled rectangle
//   1 disc   (params.y = brush roughness, params.w = seed)
//   2 ring   (params.z = stroke width in px)
//   3 cross  (an "x", params.z = stroke width in px)
//   4 dab    (a dab of the brush filling the box: ragged edges, pressed
//             wider where it lands at the top, dry streaks as it lifts;
//             params.y = roughness, params.w = seed)

struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;

struct Instance {
    @location(0) rect: vec4<f32>,   // x, y, width, height in pixels
    @location(1) color: vec4<f32>,  // sRGB + alpha
    @location(2) params: vec4<f32>, // kind, roughness, stroke, seed
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) params: vec4<f32>,
};

// Drawn as a 4-vertex triangle strip per instance.
@vertex
fn vs_main(@builtin(vertex_index) i: u32, inst: Instance) -> VsOut {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    let px = inst.rect.xy + corner * inst.rect.zw;
    let ndc = vec2<f32>(px.x / g.resolution.x * 2.0 - 1.0, 1.0 - px.y / g.resolution.y * 2.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = inst.color;
    out.local = corner * inst.rect.zw;
    out.size = inst.rect.zw;
    out.params = inst.params;
    return out;
}

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let kind = u32(in.params.x + 0.5);
    if (kind == 0u) {
        return in.color;
    }

    if (kind == 4u) {
        return dab(in);
    }

    let p = in.local - in.size * 0.5;
    let radius = min(in.size.x, in.size.y) * 0.5 - 1.0;
    // Brush edges wobble with the direction, seamlessly around the circle.
    let dir = p / max(length(p), 1e-3);
    let seed = vec2<f32>(in.params.w * 7.31, in.params.w * 3.17);
    let wobble = (noise(dir * 1.7 + seed) - 0.5) + 0.5 * (noise(dir * 4.3 + seed * 1.9) - 0.5);
    let r = radius * (1.0 + in.params.y * wobble);
    let stroke = in.params.z;

    var d = 0.0;
    if (kind == 1u) {
        d = length(p) - r;
    } else if (kind == 2u) {
        d = abs(length(p) - (r - stroke * 0.5)) - stroke * 0.5;
    } else {
        let q = abs(p);
        d = max(abs(q.x - q.y) * 0.70710678 - stroke * 0.5, max(q.x, q.y) - radius);
    }
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

// A dab of the brush: a rounded box whose sides wobble, a little wider at the
// top where the brush presses down and narrower as it lifts at the bottom,
// with dry streaks running down it that open up towards the end.
fn dab(in: VsOut) -> vec4<f32> {
    let seed = vec2<f32>(in.params.w * 7.31, in.params.w * 3.17);
    let rough = in.params.y;
    let size = in.size - 2.0; // the box has a pixel of room for the ragged edge
    let p = in.local - in.size * 0.5;
    let down = clamp(in.local.y / in.size.y, 0.0, 1.0);
    let half = vec2<f32>(size.x * 0.5 * mix(1.06, 0.86, down), size.y * 0.5);
    let r = min(size.x, size.y) * 0.28;
    // Sides: the left and right edges wander along the stroke; the top and
    // bottom along the width.
    let side = (noise(vec2<f32>(p.y * 0.35, 0.0) + seed) - 0.5) * 2.0;
    let ends = (noise(vec2<f32>(0.0, p.x * 0.5) + seed * 1.7) - 0.5) * 2.0;
    let q = abs(p) - half + r;
    var d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    d += rough * (abs(side) * 1.2 * select(0.3, 1.0, q.x > q.y) + abs(ends) * 0.9 * select(0.3, 1.0, q.y >= q.x));
    d += rough * (hash(floor(in.local) + seed) - 0.5) * 0.6;
    var alpha = clamp(0.5 - d, 0.0, 1.0);
    // Dry streaks along the stroke, more of them as the brush runs dry.
    let streak = noise(vec2<f32>(p.x * 0.9, p.y * 0.07) + seed * 2.3);
    alpha *= 1.0 - rough * 0.55 * smoothstep(0.58, 0.88, streak) * mix(0.25, 1.0, down);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
