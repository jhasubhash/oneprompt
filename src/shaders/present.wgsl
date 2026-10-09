// Final pass: copies the finished scene to the window. During a transition it
// reveals the scene over what was there before (a solid colour on startup,
// the previous frame when switching light / dark) through a growing brush
// shape: big ragged lobes, bristle streaks running the way the brush moves,
// and dry-brush grain at the edge, like ink paintings being uncovered.
//
// Each transition picks a style (how the brush travels) and a shape seed:
//   0 bloom - grows from `origin`
//   1 sweep - one broad stroke across the window at a random angle
//   2 bands - a few horizontal strokes, top to bottom, alternating direction
//   3 drops - several drops of ink landing one after another and spreading

struct Params {
    resolution: vec2<f32>,
    origin: vec2<f32>,     // where the reveal grows from, 0..1
    from_color: vec4<f32>, // rgb; w = 1 reveals over the colour, 0 over prev
    progress: f32,         // 0..1; >= 1 means no transition
    seed: f32,
    style: f32,            // see above
    shape: f32,            // per-transition random seed
};

@group(0) @binding(0) var<uniform> u: Params;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var previous: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
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

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var k = 0; k < 4; k++) {
        v += amp * noise(q);
        q = q * 2.07 + vec2<f32>(13.0, 7.0);
        amp *= 0.5;
    }
    return v;
}

// When the brush reaches this point (x, 0..1) and the way it is moving
// there (yz), for the transition's style.
fn arrival(uv: vec2<f32>, sp: vec2<f32>, aspect: f32) -> vec3<f32> {
    let q = uv * vec2<f32>(aspect, 1.0);
    let style = u32(u.style + 0.5);
    let h = fract(sin(u.shape * 12.9898) * 43758.5453);
    if (style == 1u) {
        let angle = h * 6.2831853;
        let dir = vec2<f32>(cos(angle), sin(angle));
        // Where the window's corners fall along the stroke.
        let c = vec4<f32>(0.0, dir.x * aspect, dir.y, dir.x * aspect + dir.y);
        let lo = min(min(c.x, c.y), min(c.z, c.w));
        let hi = max(max(c.x, c.y), max(c.z, c.w));
        return vec3<f32>((dot(q, dir) - lo) / (hi - lo), dir);
    }
    if (style == 2u) {
        let n = 3.0 + floor(h * 3.0);
        // Band edges wander, as hand-painted strokes do.
        let y = uv.y * n + (noise(sp * 0.004 + 7.0) - 0.5) * 0.5;
        let band = clamp(floor(y), 0.0, n - 1.0);
        let rightwards = (u32(band) % 2u) == 0u;
        let x = select(1.0 - uv.x, uv.x, rightwards);
        return vec3<f32>((band + x) / n, select(-1.0, 1.0, rightwards), 0.0);
    }
    if (style == 3u) {
        var best = 1.0e3;
        var dir = vec2<f32>(1.0, 0.0);
        for (var i = 0; i < 4; i++) {
            let k = u.shape + f32(i) * 7.31;
            let at = vec2<f32>(0.1 + 0.8 * fract(sin(k * 3.17) * 43758.5), 0.1 + 0.8 * fract(sin(k * 5.71) * 24634.6));
            let p = q - at * vec2<f32>(aspect, 1.0);
            let t = length(p) / max(aspect, 1.0) * 0.9 + f32(i) * 0.08;
            if (t < best) {
                best = t;
                dir = p / max(length(p), 1e-4);
            }
        }
        return vec3<f32>(best, dir);
    }
    let p = (uv - u.origin) * vec2<f32>(aspect, 1.0);
    // Distance to the farthest corner, so the reveal always covers the window.
    let far = length(max(abs(vec2<f32>(0.0, 0.0) - u.origin), abs(vec2<f32>(1.0, 1.0) - u.origin)) * vec2<f32>(aspect, 1.0));
    return vec3<f32>(length(p) / far, p / max(length(p), 1e-4));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.pos.xy);
    let now = textureLoad(scene, texel, 0);
    if (u.progress >= 1.0) {
        return now;
    }

    let px = in.pos.xy;
    let uv = px / u.resolution;
    let aspect = u.resolution.x / u.resolution.y;
    let sp = px + vec2<f32>(fract((u.seed + u.shape) * 0.173) * 500.0, fract((u.seed + u.shape) * 0.311) * 500.0);
    let a = arrival(uv, sp, aspect);
    let d = a.x;
    let dir = a.yz;
    // Strokes that cross the window in parts (bands) need finer raggedness.
    let rough = select(1.0, 0.35, u32(u.style + 0.5) == 2u);

    let along = vec2<f32>(dot(sp, dir), dot(sp, vec2<f32>(-dir.y, dir.x)));
    // Big lobes, then torn chunks along the edge, then a few short bristle
    // streaks running the way the brush moves, then grit.
    let lobes = (fbm(sp * 0.0032) - 0.5) * 0.6 * rough;
    let chunks = (fbm(sp * 0.018 + 31.0) - 0.5) * 0.14 * rough;
    let bristles = (noise(along * vec2<f32>(0.02, 0.16)) - 0.5) * 0.06;
    let grit = (hash(px) - 0.5) * 0.015;
    let edge = d + lobes + chunks + bristles + grit;

    // Front runs from fully hidden (progress 0) to fully shown (progress 1).
    let front = u.progress * 1.95 - 0.45;
    let shown = smoothstep(edge - 0.004, edge + 0.004, front);

    var before = textureLoad(previous, texel, 0);
    if (u.from_color.w > 0.5) {
        before = vec4<f32>(u.from_color.rgb, 1.0);
    }
    return mix(before, now, shown);
}
