// Final pass: copies the finished scene to the window. During a transition it
// reveals the scene over what was there before (a solid colour on startup,
// the previous frame when switching light / dark) through a growing brush
// shape: big ragged lobes, bristle streaks running the way the brush moves,
// and dry-brush grain at the edge, like ink paintings being uncovered.
//
// The brushwork is made up at random for every transition (animation.rs,
// `Brush`): a few strokes and drops, each reaching a point at some time; the
// earliest of them reveals it, and ink soaks outward from them to cover the
// rest. Raggedness, lobe size and bristles vary per transition too.

struct Params {
    resolution: vec2<f32>,
    progress: f32,         // 0..1; >= 1 means no transition
    seed: f32,             // the paper's session seed
    from_color: vec4<f32>, // rgb; w = 1 reveals over the colour, 0 over prev
    edge: vec4<f32>,       // lobes, chunks, bristles, lobe size
    info: vec4<f32>,       // marks, 1 / latest arrival, noise seed, soak speed
    // Two per mark: (x, y, angle, half width), (delay, speed, bend, drop).
    marks: array<vec4<f32>, 16>,
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

// When the brush reaches `q` (x scaled by the aspect ratio), 0..1; the way
// the winning mark moves there, for the bristle streaks; and whether it is a
// drop (whose edge has no bristles: radial streaks look like a burr).
fn arrival(q: vec2<f32>) -> vec4<f32> {
    var best = 1.0e6;
    var dir = vec2<f32>(1.0, 0.0);
    var drop = 0.0;
    let count = u32(u.info.x + 0.5);
    let soak = u.info.w;
    for (var i = 0u; i < count; i++) {
        let a = u.marks[2u * i];
        let b = u.marks[2u * i + 1u];
        let d = q - a.xy;
        var t = 0.0;
        var way = vec2<f32>(1.0, 0.0);
        if (b.w > 0.5) {
            // A drop spreads out in every direction.
            t = b.x + length(d) / b.y;
            way = d / max(length(d), 1e-4);
        } else {
            // A stroke: the tip passes `along` at its speed; past its sides,
            // or behind where it started, ink arrives by soaking.
            way = vec2<f32>(cos(a.z), sin(a.z));
            let along = dot(d, way);
            let across = dot(d, vec2<f32>(-way.y, way.x)) - b.z * along * along;
            t = b.x + max(along, 0.0) / b.y + (max(abs(across) - a.w, 0.0) + max(-along, 0.0)) / soak;
        }
        if (t < best) {
            best = t;
            dir = way;
            drop = b.w;
        }
    }
    return vec4<f32>(best * u.info.y, dir, drop);
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
    let shape = u.info.z;
    let sp = px + vec2<f32>(fract((u.seed + shape) * 0.173) * 500.0, fract((u.seed + shape) * 0.311) * 500.0);
    let a = arrival(uv * vec2<f32>(aspect, 1.0));
    let d = a.x;
    let dir = a.yz;

    let along = vec2<f32>(dot(sp, dir), dot(sp, vec2<f32>(-dir.y, dir.x)));
    // Big lobes, then torn chunks along the edge, then a few short bristle
    // streaks running the way the brush moves, then grit.
    let lobes = (fbm(sp * u.edge.w) - 0.5) * u.edge.x;
    let chunks = (fbm(sp * 0.018 + 31.0) - 0.5) * u.edge.y;
    let bristles = (noise(along * vec2<f32>(0.02, 0.16)) - 0.5) * u.edge.z * (1.0 - 0.85 * a.w);
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
