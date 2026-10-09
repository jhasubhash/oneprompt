// Procedural backgrounds. Everything is computed per pixel from noise, so the
// texture is resolution independent and costs no image assets.

struct Uniforms {
    surface: vec4<f32>,        // rgb + texture kind in w
    surface_shade: vec4<f32>,  // rgb + texture strength in w
    resolution: vec2<f32>,
    time: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

// One triangle that covers the whole screen.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
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

fn vignette(uv: vec2<f32>) -> f32 {
    let d = distance(uv, vec2<f32>(0.5));
    return smoothstep(0.35, 0.85, d);
}

fn paper(px: vec2<f32>, uv: vec2<f32>) -> f32 {
    // Long, stretched noise reads as fibers; fine noise reads as grain.
    let fibers = fbm(px * vec2<f32>(0.012, 0.09));
    let blotches = fbm(px * 0.004);
    let grain = hash(px) - 0.5;
    return fibers * 0.35 + blotches * 0.45 + grain * 0.08 + vignette(uv) * 0.6;
}

fn chalkboard(px: vec2<f32>, uv: vec2<f32>) -> f32 {
    // Broad smudges from old erasing plus speckled dust.
    let smudge = fbm(px * vec2<f32>(0.003, 0.006));
    let dust = step(0.985, hash(floor(px * 0.7)));
    return smudge * 0.7 + dust * 0.5 - vignette(uv) * 0.4;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = in.pos.xy;
    let uv = px / u.resolution;
    let kind = u32(u.surface.w);
    let strength = u.surface_shade.w;

    var t = 0.0;
    if (kind == 1u) {
        t = paper(px, uv);
    } else if (kind == 2u) {
        t = chalkboard(px, uv);
    }

    let color = mix(u.surface.rgb, u.surface_shade.rgb, clamp(t * strength, 0.0, 1.0));
    return vec4<f32>(color, 1.0);
}
