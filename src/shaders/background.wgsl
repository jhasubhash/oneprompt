// Procedural backgrounds. Everything is computed per pixel from noise, so the
// texture is resolution independent and costs no image assets.

struct Uniforms {
    surface: vec4<f32>,        // rgb + texture kind in w
    surface_shade: vec4<f32>,  // rgb + texture strength in w
    accent: vec4<f32>,         // rgb of ink splatter + amount in w
    resolution: vec2<f32>,
    seed: f32,                 // per-session: every launch gets a new sheet
    scale: f32,                // physical pixels per logical pixel
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

fn rotate(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

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

// Hand-made washi: a rough, lumpy tooth lit from the top left, mottled pulp,
// long fibers, and the hard-edged tide lines water leaves when it dries.
fn washi(px_in: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    // The session seed shifts every pattern, so each launch is a new sheet.
    let px = px_in + vec2<f32>(fract(u.seed * 0.137) * 400.0, fract(u.seed * 0.291) * 400.0);

    // Dark sheets (konshi) are shaded towards a lighter tone, so their
    // vignette has to darken separately instead of going through `shade`.
    let dark_sheet = dot(u.surface.rgb, vec3<f32>(0.3, 0.59, 0.11)) < 0.5;
    let clouds = fbm(px * 0.006);
    let mottle = noise(px * 0.05) * 0.6 + noise(px * 0.13) * 0.4;
    let grain = hash(px) - 0.5;
    var shade = clouds * 0.3 + mottle * 0.12 + grain * 0.08;
    if (!dark_sheet) {
        // Parchment browns heavily towards the corners, as in the paintings.
        shade += vignette(uv) * 0.75;
    }

    // Water stains: regions of a slow noise field. Pigment collects at the
    // edge where the water stopped: a thin, dark, slightly ragged tide line
    // with a faint wash inside.
    let field = fbm(px * 0.0016 + vec2<f32>(3.7, 1.3)) + (noise(px * 0.08) - 0.5) * 0.012;
    for (var k = 0; k < 3; k++) {
        let level = 0.58 + f32(k) * 0.045;
        let inside = field - level;
        let edge = smoothstep(-0.002, 0.0, inside);
        shade += edge * (0.06 + 0.5 * exp(-max(inside, 0.0) * 260.0));
    }

    var color = mix(u.surface.rgb, u.surface_shade.rgb, clamp(shade * u.surface_shade.w, 0.0, 1.0));

    // Relief: light the tooth from the top left so the sheet reads as rough.
    let e = 0.75;
    let dx = tooth(px + vec2<f32>(e, 0.0)) - tooth(px - vec2<f32>(e, 0.0));
    let dy = tooth(px + vec2<f32>(0.0, e)) - tooth(px - vec2<f32>(0.0, e));
    let lit = clamp(-(dx + dy) * 1.6, -1.0, 1.0);
    color = color * (1.0 + lit * select(0.11, 0.3, dark_sheet));
    if (dark_sheet) {
        color = color * (1.0 - vignette(uv) * 0.35);
    }

    // Fibers: mostly lighter than the sheet, a few darker ones.
    var light = 0.0;
    for (var k = 0; k < 3; k++) {
        light += fiber(px, 0.5 + f32(k) * 1.9, f32(k) * 21.0);
    }
    color = mix(color, mix(color, vec3<f32>(1.0), select(0.1, 0.045, dark_sheet)), clamp(light, 0.0, 1.0));
    let dark = fiber(px, 2.4, 57.0);
    color = mix(color, u.surface_shade.rgb * 0.8, dark * select(0.2, 0.08, dark_sheet));

    // A few flicks of ink splatter in the top-right corner, well away from
    // where the text starts.
    let corner = smoothstep(0.55, 1.0, uv.x) * smoothstep(0.45, 0.0, uv.y);
    let cell = floor(px / 9.0);
    let h = hash(cell + 0.5);
    if (h > 1.0 - 0.06 * corner * u.accent.w) {
        let center = (cell + vec2<f32>(hash(cell + 1.3), hash(cell + 2.9))) * 9.0;
        let radius = 0.6 + 2.4 * hash(cell + 4.1) * hash(cell + 7.7);
        let dot_mask = 1.0 - smoothstep(radius - 0.6, radius + 0.6, distance(px, center));
        color = mix(color, u.accent.rgb, dot_mask * 0.8);
    }
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Textures are designed in logical pixels so they look the same on any display.
    let px = in.pos.xy / max(u.scale, 1.0);
    let uv = in.pos.xy / u.resolution;
    let kind = u32(u.surface.w);
    let strength = u.surface_shade.w;

    if (kind == 3u) {
        return vec4<f32>(washi(px, uv), 1.0);
    }

    var t = 0.0;
    if (kind == 1u) {
        t = paper(px, uv);
    } else if (kind == 2u) {
        t = chalkboard(px, uv);
    }

    let color = mix(u.surface.rgb, u.surface_shade.rgb, clamp(t * strength, 0.0, 1.0));
    return vec4<f32>(color, 1.0);
}
