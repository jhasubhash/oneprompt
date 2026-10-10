// Tsushima's reveals. The window opens through a growing brush shape: big
// ragged lobes, bristle streaks running the way the brush moves, and
// dry-brush grain at the edge, like ink paintings being uncovered.
//
// The brushwork is made up at random for every transition (brush.rs,
// `Brush`): a few strokes and drops, each reaching a point at some time; the
// earliest of them reveals it, and ink soaks outward from them to cover the
// rest. Raggedness, lobe size and bristles vary per transition too.
//
// Switching to Tsushima (light / dark, or from another theme) is cut in
// with a sword instead: thin tapered slashes drawn across the window in an
// instant, droplets flung off them, a glint along the fresh cut and its
// edges in seal ink; then a breath, and the new look runs out of every cut
// and droplet like watercolour on damp paper: fast at first and slowing,
// its front fingering out and blooming, pigment pooling in a tide line
// along it and settling into the grain behind it, and the paper just ahead
// darkened by the water reaching it first.

// Closing a sheet burns it away (brush.rs, `PlanData::burn`): a fire lit
// at marks[0].xy spreads to marks[0].z by the end, its front ragged and
// fingering out along the paper, the sheet ahead of it browning and
// charring, the edge glowing, ash and embers behind it fading to what is
// underneath: the next tab, or (the window closing) ash.

// What brush.rs describes (`PlanData`).
struct Plan {
    edge: vec4<f32>,       // lobes, chunks, bristles, lobe size
    info: vec4<f32>,       // marks, 1 / latest arrival, noise seed, soak speed
    cut: vec4<f32>,        // rgb of cut edges, strength
    front: vec4<f32>,      // the front runs progress * x + y; z: wash slowing
    // Three per mark: (x, y, angle, half width), (delay, speed, bend, kind:
    // 0 stroke, 1 drop, 2 cut), (cut length, hold, droplet side, splatter).
    marks: array<vec4<f32>, 24>,
};

@group(0) @binding(2) var<uniform> plan: Plan;

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

const PI: f32 = 3.14159265;

// How long the watercolour takes to run `distance` past a cut (already
// divided by the soak speed): it slows as it spreads (animation.rs, `wash`).
fn wash(distance: f32) -> f32 {
    return distance * (1.0 + plan.front.z * distance);
}

// Where a point's ink comes from: when it arrives (before normalising), the
// way the winning mark moves (for the bristle streaks), what kind of mark it
// is, when a cut opens up (its own raw time), and for the inside of a cut or
// droplet, how far the point is from its rim (window heights; -1 elsewhere).
struct Hit {
    t: f32,
    dir: vec2<f32>,
    kind: f32,
    open: f32,
    inner: f32,
};

// Droplets flung off a cut, one layer of them: in cells along the blade's
// path, some cells throw a droplet, which lands off to one side, flies a
// little later the further it goes, and is streaked out the way the blade
// moved. Returns the earliest arrival over the droplets near `along`.
fn droplets(hit: ptr<function, Hit>, along: f32, across: f32, i: u32, layer: f32,
            cell: f32, chance: f32, reach: f32, small: f32, large: f32, stretch: f32) {
    let a = plan.marks[3u * i];
    let b = plan.marks[3u * i + 1u];
    let c = plan.marks[3u * i + 2u];
    let span = c.x;
    let open = b.x + span / b.y + c.y;
    let k0 = floor(along / cell);
    for (var j = -1; j <= 1; j++) {
        let k = vec2<f32>(k0 + f32(j), f32(i) * 13.0 + layer * 7.0 + plan.info.z);
        if (hash(k) > chance * c.w) {
            continue;
        }
        let at = (k.x + hash(k + 1.7)) * cell;
        if (at < 0.0 || at > span) {
            continue;
        }
        let far = hash(k + 3.1);
        let side = select(c.z, -c.z, hash(k + 5.3) < 0.15);
        let width = a.w * sin(PI * at / span);
        let off = side * (width + 0.003 + reach * far * far);
        let r = mix(small, large, pow(hash(k + 7.9), 2.0)) * (1.0 - 0.6 * far);
        let q = vec2<f32>((along - at) / (1.0 + stretch * far), across - off);
        let wobble = (noise(vec2<f32>(along, across) * 250.0 + k) - 0.5) * r * 0.5;
        let dist = length(q) + wobble;
        let landed = b.x + at / b.y + abs(off) * 0.25;
        // Once the cut opens, a droplet spreads too, but only so far: only
        // the cells next to the point's are looked at, so a wider blot would
        // be clipped square by the ones further away.
        let spread = length(vec2<f32>(along - at, across - off)) - r;
        var t = 1.0e6;
        if (dist < r) {
            t = landed;
        } else if (spread < cell * 0.4) {
            t = open + wash(max(spread, 0.0) / plan.info.w);
        }
        if (t < (*hit).t) {
            (*hit).t = t;
            (*hit).dir = vec2<f32>(cos(a.z), sin(a.z));
            (*hit).kind = 2.0;
            (*hit).open = open;
            (*hit).inner = select(-1.0, r - dist, dist < r);
        }
    }
}

// When the brush reaches `q` (x scaled by the aspect ratio).
fn arrival(q: vec2<f32>) -> Hit {
    var hit = Hit(1.0e6, vec2<f32>(1.0, 0.0), 0.0, 0.0, -1.0);
    let count = u32(plan.info.x + 0.5);
    let soak = plan.info.w;
    for (var i = 0u; i < count; i++) {
        let a = plan.marks[3u * i];
        let b = plan.marks[3u * i + 1u];
        let c = plan.marks[3u * i + 2u];
        let d = q - a.xy;
        var t = 0.0;
        var way = vec2<f32>(1.0, 0.0);
        var open = 0.0;
        var inner = -1.0;
        if (b.w > 1.5) {
            // A cut: the blade passes in an instant, widest mid-way and
            // tapering to nothing at both ends; the rest waits for the cut to
            // open, then soaks out from it.
            way = vec2<f32>(cos(a.z), sin(a.z));
            let along = dot(d, way);
            let across = dot(d, vec2<f32>(-way.y, way.x)) - b.z * along * along;
            let span = c.x;
            let width = a.w * sin(PI * clamp(along / span, 0.0, 1.0));
            open = b.x + span / b.y + c.y;
            let off = max(abs(across) - width, 0.0) + max(along - span, 0.0) + max(-along, 0.0);
            if (off <= 0.0) {
                t = b.x + along / b.y;
                inner = width - abs(across);
            } else {
                t = open + wash(off / soak);
            }
            if (t < hit.t) {
                hit = Hit(t, way, 2.0, open, inner);
            }
            // Big drops close to the cut, and a fine spray further out.
            droplets(&hit, along, across, i, 0.0, 0.045, 0.55, 0.16, 0.003, 0.014, 2.5);
            droplets(&hit, along, across, i, 1.0, 0.012, 0.5, 0.08, 0.0012, 0.0035, 1.5);
            continue;
        }
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
        if (t < hit.t) {
            hit = Hit(t, way, b.w, 0.0, -1.0);
        }
    }
    return hit;
}

fn theme_transition(px: vec2<f32>, now: vec4<f32>, before_in: vec4<f32>) -> vec4<f32> {
    if (plan.front.w > 0.5) {
        return burn(px, now, before_in);
    }
    let uv = px / frame.resolution;
    let aspect = frame.resolution.x / frame.resolution.y;
    let shape = plan.info.z;
    let sp = px + vec2<f32>(fract((frame.seed + shape) * 0.173) * 500.0, fract((frame.seed + shape) * 0.311) * 500.0);
    let hit = arrival(uv * vec2<f32>(aspect, 1.0));
    let d = hit.t * plan.info.y;
    let dir = hit.dir;
    let is_cut = hit.kind > 1.5;
    let is_drop = hit.kind > 0.5 && !is_cut;
    let open = hit.open * plan.info.y;

    // A cut and its droplets stay crisp; the edge grows ragged only as it
    // soaks out once the cut has opened.
    var rough = 1.0;
    if (is_cut) {
        rough = smoothstep(0.0, 0.25, d - open);
    }

    let along = vec2<f32>(dot(sp, dir), dot(sp, vec2<f32>(-dir.y, dir.x)));
    // Big lobes, then torn chunks along the edge, then a few short bristle
    // streaks running the way the brush moves, then grit.
    let lobes = (fbm(sp * plan.edge.w) - 0.5) * plan.edge.x;
    let chunks = (fbm(sp * 0.018 + 31.0) - 0.5) * plan.edge.y;
    let bristles = (noise(along * vec2<f32>(0.02, 0.16)) - 0.5) * plan.edge.z * select(1.0, 0.15, is_drop);
    let grit = (hash(px) - 0.5) * 0.015;

    // How far the watercolour has run past the cut it came from: its front
    // grows more irregular the further it goes. 0 everywhere but in a wash.
    var wet = 0.0;
    if (is_cut) {
        wet = smoothstep(0.0, 0.08, d - open);
    }
    // Fingers where the water follows the paper's fibres (warped, so they
    // curl), and scalloped blooms bulging out of the front.
    let warp = vec2<f32>(fbm(sp * 0.003 + 11.0), fbm(sp * 0.003 + 23.0)) * 160.0;
    let fingers = (fbm((sp + warp) * vec2<f32>(0.007, 0.016) + 5.0) - 0.5) * 0.2;
    let blooms = -abs(noise(sp * 0.02 + 41.0) - 0.5) * 0.06;
    // Pigment settling in clumps instead of brush grit.
    let granules = (noise(px * 0.22 + 17.0) - 0.5) * 0.006;
    // Water rounds the front off: no torn chunks or bristles in a wash.
    let smooth_edge = d + (lobes + (chunks + bristles) * (1.0 - wet)) * rough + (fingers + blooms) * wet;
    let edge = smooth_edge + mix(grit * max(rough, 0.3), granules, wet);
    // How far the edge moves per pixel (without the grit), to draw cut
    // edges a few pixels wide.
    let per_pixel = max(fwidth(smooth_edge), 1e-6);

    // Front runs from fully hidden (progress 0) to fully shown (progress 1).
    let front = frame.progress * plan.front.x + plan.front.y;
    // Whatever is left at the very end fades in rather than popping.
    // A wash's front is a sharp line a couple of pixels wide, so its tide
    // line sits right against it.
    let soft = mix(0.004, 1.2 * clamp(per_pixel, 1e-5, 0.004), wet);
    let shown = max(smoothstep(edge - soft, edge + soft, front), smoothstep(0.9, 1.0, frame.progress));

    var before = before_in;
    var after = now;
    if (wet > 0.0) {
        // Everything the water does dries off by the end, leaving the new
        // look exactly as it is.
        let fresh = wet * (1.0 - smoothstep(0.75, 1.0, frame.progress));
        // Distances from the front in pixels, scaled with the window so the
        // look is the same at any size.
        let to_px = 1.0 / clamp(per_pixel, 1e-5, 0.01);
        let size = frame.resolution.y / 700.0;
        let ahead = (edge - front) * to_px / size;
        let behind = max(-ahead, 0.0);
        // Pigment: more of the same colour, deeper and richer (parchment
        // pools into umber, the dark sheet into black).
        let pigment = pow(now.rgb, vec3<f32>(2.6)) * 0.85;
        // A tide line where the front is, a damp, deeper wash behind it that
        // lightens as it dries, both settling unevenly into the grain.
        let settle = 0.6 + 0.8 * noise(px * 0.18 + 3.0);
        let pool = exp(-behind / 5.0) * 0.75 + exp(-behind / 90.0) * 0.25;
        after = vec4<f32>(mix(now.rgb, pigment, clamp(pool * settle * fresh, 0.0, 1.0)), 1.0);
        // Ahead of the front the water arrives first: the old look darkens
        // where the paper is damp, and faint feathers of colour bleed into it.
        let damp = (1.0 - smoothstep(0.0, 50.0, ahead)) * fresh;
        let feather = (1.0 - smoothstep(0.0, 14.0, ahead + (fbm(sp * 0.05 + 7.0) - 0.5) * 30.0)) * fresh;
        before = vec4<f32>(mix(before.rgb * (1.0 - 0.16 * damp), now.rgb, feather * 0.3), 1.0);
    }
    var color = mix(before, after, shown);

    if (is_cut) {
        // Seal ink along the rim of every cut and droplet, and along the edge
        // as it starts to soak out, fading as the cuts open up.
        let rim_width = max(1.5, frame.resolution.y / 520.0);
        var behind = (front - edge) / per_pixel;
        // Where a cut or droplet meets the ink soaking out of it, arrival
        // jumps from one pixel to the next: that is no edge, so no rim.
        if (per_pixel > 0.02) {
            behind = 1.0e6;
        }
        if (hit.inner >= 0.0) {
            behind = hit.inner * frame.resolution.y;
        }
        // Rims inside the opened-up area go first, so they do not linger as
        // lines in the middle of the new look.
        let fade = select(1.0 - smoothstep(0.05, 0.4, front - open), 1.0 - smoothstep(0.0, 0.04, front - open), hit.inner >= 0.0);
        // Once the colour runs, its own tide line takes over from the rim.
        let rim = shown * (1.0 - smoothstep(rim_width * 0.6, rim_width * 1.4, behind)) * fade * (1.0 - wet) * plan.cut.w;
        color = vec4<f32>(mix(color.rgb, plan.cut.rgb, rim), 1.0);
        // The blade's glint: a flash of light along the cut as it is made.
        if (hit.inner >= 0.0 && d < open) {
            let glint = (1.0 - smoothstep(0.0, 0.07, front - d)) * shown;
            let core = smoothstep(0.0, 0.5, hit.inner * frame.resolution.y / max(rim_width * 3.0, 1.0));
            color = vec4<f32>(mix(color.rgb, vec3<f32>(1.0, 0.97, 0.9), glint * core * 0.75), 1.0);
        }
    }
    return color;
}
// The burn. Distances are in window heights.
fn burn(px: vec2<f32>, now_in: vec4<f32>, before: vec4<f32>) -> vec4<f32> {
    let aspect = frame.resolution.x / frame.resolution.y;
    let p = px / frame.resolution * vec2<f32>(aspect, 1.0);
    let seed = vec2<f32>(plan.info.z * 1.7, plan.info.z * 0.9);
    let lp = px / max(frame.scale, 1.0);
    // The front fingers out where the paper burns faster.
    let ragged = (fbm(p * 3.2 + seed) - 0.5) * 0.42 + (noise(lp * 0.12 + seed) - 0.5) * 0.05;
    let dist = length(p - plan.marks[0].xy) + ragged;
    let front = frame.progress * plan.marks[0].z;
    let behind = front - dist;
    let edge = 0.035;

    var under = now_in;
    let ash_rgb = vec3<f32>(0.07, 0.055, 0.045) * (0.7 + 0.6 * noise(lp * 0.3 + seed));
    if (frame.closing > 0.5) {
        under = vec4<f32>(ash_rgb, 1.0);
    }
    let flicker = 0.65 + 0.35 * noise(lp * 0.2 + vec2<f32>(frame.time * 6.0, 0.0) + seed);
    if (behind > edge) {
        // Burnt through: charred flakes and embers cling for a moment, then
        // what is underneath shows.
        let ash = 1.0 - smoothstep(edge, edge + 0.16, behind);
        var rgb = mix(under.rgb, ash_rgb, ash * 0.85);
        let speck = step(0.965, hash(floor(lp * 0.5) + seed)) * ash;
        rgb = mix(rgb, vec3<f32>(1.0, 0.45, 0.1), speck * flicker);
        return vec4<f32>(rgb, 1.0);
    }
    if (behind > 0.0) {
        // The burning edge: glowing orange at its leading side, deep red
        // and black behind.
        let t = behind / edge;
        let hot = mix(vec3<f32>(1.0, 0.72, 0.28), vec3<f32>(0.62, 0.1, 0.03), t);
        let rgb = mix(hot * flicker, ash_rgb, smoothstep(0.55, 1.0, t));
        return vec4<f32>(rgb, 1.0);
    }
    // Ahead of it the paper browns and then chars as the heat reaches it.
    let ahead = -behind;
    let scorch = 1.0 - smoothstep(0.0, 0.14, ahead);
    var rgb = mix(before.rgb, before.rgb * vec3<f32>(0.62, 0.42, 0.22), scorch * 0.8);
    rgb = mix(rgb, vec3<f32>(0.08, 0.05, 0.03), 1.0 - smoothstep(0.0, 0.018, ahead));
    return vec4<f32>(rgb, 1.0);
}
