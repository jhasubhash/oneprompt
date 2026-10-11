// The transition stage: copies the finished scene to the window, or while a
// reveal or switch runs, lets the theme's `theme_transition` show it over
// what was there before (a solid colour on opening, the last frame shown
// when switching).

@group(0) @binding(3) var scene: texture_2d<f32>;
@group(0) @binding(4) var previous: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> FullScreen {
    return full_screen(i);
}

@fragment
fn fs_main(in: FullScreen) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.pos.xy);
    let now = textureLoad(scene, texel, 0);
    if (frame.progress >= 1.0) {
        return now;
    }
    var before = textureLoad(previous, texel, 0);
    if (frame.from_color.w > 0.5) {
        before = vec4<f32>(frame.from_color.rgb, 1.0);
    }
    return theme_transition(in.pos.xy, now, before);
}

// Writing that crumbles into dust and is blown away, for a theme's clear:
// a ragged front sweeps across the text from its right to its left over the
// first half of the transition, slanting so the bottom goes a little ahead;
// behind it the ink breaks up, a speck to each `cell` (physical pixels), and
// the specks are blown off up and to the right, gathering speed, spreading
// out, shrinking and fading over the rest. The ink is what the frame before
// has that the scene after doesn't, so the paper beneath stays put; the
// theme colours the result its own way.
//
// The specks are gathered, not scattered: they travel in a few layers, each
// with its own wind (speed and heading), so for each layer the speck that
// could be over a pixel is found by solving its flight backwards, then the
// specks around that point are checked exactly.

struct DustArea {
    lo: vec2<f32>,   // the text's top left, physical pixels
    hi: vec2<f32>,   // its bottom right
    cell: f32,       // spacing of the specks, physical pixels
    travel: f32,     // how far the fastest blow, physical pixels
    seed: f32,
};

struct Dust {
    ink: vec3<f32>,  // the specks over this pixel: add it to the scene after
    heat: f32,       // how freshly they broke off, 0..1 (1: just now)
    intact: f32,     // 1: the writing here is still whole (show the frame before)
    near: f32,       // how close the front is to it, 0..1 (1: about to go)
};

const DUST_SWEEP: f32 = 0.5;
const DUST_FLIGHT: f32 = 0.45;
const DUST_LAYERS: i32 = 6;

// When the front reaches `s` (progress), smooth across the text and ragged
// in clumps; `chip` breaks it up per speck.
fn dust_release(s: vec2<f32>, a: DustArea, chip: f32) -> f32 {
    let slant = 0.35;
    let size = a.hi - a.lo;
    let across = ((a.hi.x - s.x) + (a.hi.y - s.y) * slant) / max(size.x + size.y * slant, 1.0);
    let ragged = (noise(s / (a.cell * 7.0) + a.seed) - 0.5) * 0.08;
    return clamp(across, 0.0, 1.0) * DUST_SWEEP + ragged + (chip - 0.5) * 0.02;
}

// Layer `k`'s wind: (heading, speed).
fn dust_wind(k: i32, a: DustArea) -> vec2<f32> {
    let h = vec2<f32>(f32(k) * 3.7 + 1.3, a.seed);
    return vec2<f32>(-0.2 + (hash(h) - 0.5) * 1.0, 0.6 + 0.8 * hash(h + 5.1));
}

// How far a speck from `s` has been blown after `u` of its flight (0..1).
fn dust_flight(s: vec2<f32>, u: f32, wind: vec2<f32>, a: DustArea) -> vec2<f32> {
    let way = vec2<f32>(cos(wind.x), sin(wind.x));
    // Eddies: neighbouring specks swirl alike, so the dust moves in plumes.
    let swirl = (noise(s / (a.cell * 22.0) + a.seed * 1.7 + wind.x) - 0.5) * 0.3;
    let blown = way * wind.y * (0.3 * u + 0.7 * u * u) + vec2<f32>(-way.y, way.x) * swirl * u;
    return blown * a.travel;
}

fn dust_age(s: vec2<f32>, a: DustArea, chip: f32) -> f32 {
    return (frame.progress - dust_release(s, a, chip)) / DUST_FLIGHT;
}

// What ink a speck from `s` carries: the old frame's ink there, where it had
// more than the scene after (not what the scene after adds).
fn dust_ink(s: vec2<f32>, a: DustArea) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        let o = (vec2<f32>(f32(k & 1), f32(k >> 1u)) - 0.5) * a.cell * 0.5;
        let at = vec2<i32>(clamp(s + o, vec2<f32>(0.0), frame.resolution - 1.0));
        let old = textureLoad(previous, at, 0).rgb;
        let clean = textureLoad(scene, at, 0).rgb;
        let more = distance(old, frame.surface.rgb) > distance(clean, frame.surface.rgb) + 0.02;
        sum += select(vec3<f32>(0.0), old - clean, more);
    }
    return sum * 0.25;
}

fn dust(px: vec2<f32>, a: DustArea) -> Dust {
    var out = Dust(vec3<f32>(0.0), 0.0, 0.0, 0.0);
    let t = frame.progress;
    let own = floor(px / a.cell);
    let release = dust_release(px, a, hash(own + a.seed));
    if (t < release) {
        // Not reached yet, and nothing blown from the right of it can
        // land here: the dust only moves away from the front.
        out.intact = 1.0;
        out.near = 1.0 - smoothstep(0.0, 0.035, release - t);
        return out;
    }
    for (var k = 0; k < DUST_LAYERS; k++) {
        let wind = dust_wind(k, a);
        // Where a speck over this pixel set off from: the further left, the
        // later it broke off, so it has been blown less far; bisect.
        var left = px.x - a.travel * (wind.y + 0.25) - a.cell * 2.0;
        var right = px.x + a.travel * 0.1;
        var start = px;
        for (var i = 0; i < 11; i++) {
            let mid = 0.5 * (left + right);
            var s = vec2<f32>(mid, px.y);
            let u0 = clamp(dust_age(s, a, 0.5), 0.0, 1.0);
            s.y = px.y - dust_flight(s, u0, wind, a).y;
            let u = clamp(dust_age(s, a, 0.5), 0.0, 1.0);
            let lands = mid + dust_flight(s, u, wind, a).x;
            if (lands < px.x) {
                left = mid;
            } else {
                right = mid;
            }
            start = s;
        }
        // The specks around there, exactly: those of this layer still in flight.
        let base = floor(start / a.cell);
        for (var j = 0; j < 9; j++) {
            let c = base + vec2<f32>(f32(j % 3 - 1), f32(j / 3 - 1));
            let chip = hash(c + a.seed);
            if (i32(hash(c + a.seed + 17.3) * f32(DUST_LAYERS)) != k) {
                continue;
            }
            let s = (c + 0.5) * a.cell;
            if (any(s < a.lo) || any(s > a.hi)) {
                continue;
            }
            let u = dust_age(s, a, chip);
            if (u <= 0.0 || u >= 1.0) {
                continue;
            }
            let at = s + dust_flight(s, u, wind, a);
            let radius = a.cell * (0.3 + 0.35 * hash(c + 2.9)) * (1.0 - 0.55 * u);
            let cover = 1.0 - smoothstep(radius - 0.8, radius + 0.8, distance(px, at));
            if (cover <= 0.0) {
                continue;
            }
            let ink = dust_ink(s, a);
            let fade = 1.0 - smoothstep(0.45, 1.0, u);
            out.ink += ink * cover * fade * 1.6;
            out.heat = max(out.heat, pow(1.0 - u, 3.0) * cover * clamp(length(ink) * 4.0, 0.0, 1.0));
        }
    }
    return out;
}
