// Logbook's transitions: a page turning. The fold sweeps across the window
// (right to left, or back), slightly tilted. Beyond it, still to turn, is
// the old page; behind it the new one shows, shaded where the turning page
// still shadows it; at the fold the page's underside curls over, lit along
// its ridge. Closing the window turns to the inside of the cover.

// What mod.rs describes (`PlanData`).
struct Plan {
    way: f32,    // 1: turning right to left, -1: left to right
    tilt: f32,   // of the fold (or the tear), radians
    seed: f32,
    mode: f32,   // 0: a page turns; 1: the pane burns; 2: the pane is torn out
    at_x: f32,   // the burn: where it starts (x across 0..aspect, y down 0..1)
    at_y: f32,
    reach: f32,  // how far its front has gone when done
    _pad: f32,
    area: vec4<f32>, // the pane (x, y, w, h, 0..1 of the window)
};

@group(0) @binding(2) var<uniform> plan: Plan;

fn theme_transition(px: vec2<f32>, now_in: vec4<f32>, before: vec4<f32>) -> vec4<f32> {
    if (plan.mode > 0.5) {
        let uv = px / frame.resolution;
        if (any(uv < plan.area.xy) || any(uv > plan.area.xy + plan.area.zw)) {
            return now_in;
        }
        if (plan.mode > 1.5) {
            return torn(px, now_in);
        }
        return burnt(px, now_in, before);
    }
    let res = frame.resolution;
    let aspect = res.x / res.y;
    var now = now_in;
    if (frame.closing > 0.5) {
        // The inside of the cover: dark marbled board.
        let p = px / max(frame.scale, 1.0) * 0.01;
        let marble = fbm(p * 3.0 + vec2<f32>(fbm(p * 2.0) * 2.0, 0.0));
        now = vec4<f32>(mix(vec3<f32>(0.12, 0.08, 0.06), vec3<f32>(0.3, 0.12, 0.08), marble), 1.0);
    }
    // Position across the page, along the way it turns: 0 where it starts,
    // 1 where it ends.
    let uv = px / res;
    let q = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
    let r = rotate(q, plan.tilt);
    var x = r.x / aspect + 0.5;
    if (plan.way > 0.0) {
        x = 1.0 - x;
    }
    // The fold travels from before the start to past the end.
    let fold = frame.progress * 1.35 - 0.15;
    let curl = 0.12;
    if (x < fold - curl) {
        // Turned: the new page, shadowed near the fold.
        let shade = 1.0 - 0.35 * (1.0 - smoothstep(0.0, 0.12, fold - curl - x));
        return vec4<f32>(now.rgb * shade, 1.0);
    }
    if (x < fold) {
        // The curl: the page's underside, folded over and lit along its
        // ridge, a little of the old page showing through the paper.
        // The turning page's back: the old page's own paper, mirrored
        // across the fold, its writing showing through only faintly.
        let t = (fold - x) / curl;
        let ridge = sin(t * 3.14159265);
        let back = 2.0 * (fold - x);
        let sign = select(1.0, -1.0, plan.way > 0.0);
        let dq = rotate(vec2<f32>(back * sign * aspect, 0.0), -plan.tilt);
        let mirrored = clamp(px + vec2<f32>(dq.x / aspect * res.x, dq.y * res.y), vec2<f32>(0.0), res - 1.0);
        // The paper's own tone, without the writing: of a few samples, the
        // lightest on a light page, the darkest on a dark one.
        let old = textureLoad(previous, vec2<i32>(mirrored), 0).rgb;
        let light_page = dot(before.rgb, vec3<f32>(0.3, 0.59, 0.11)) > 0.4;
        var tone = old;
        for (var k = 0; k < 4; k++) {
            let o = vec2<f32>(cos(f32(k) * 1.5708), sin(f32(k) * 1.5708)) * 4.0 * frame.scale;
            let s = textureLoad(previous, vec2<i32>(clamp(mirrored + o, vec2<f32>(0.0), res - 1.0)), 0).rgb;
            tone = select(min(tone, s), max(tone, s), light_page);
        }
        let paper = mix(tone, old, 0.12) * (0.72 + 0.38 * ridge);
        return vec4<f32>(paper, 1.0);
    }
    // Still to turn: the old page, lifting slightly ahead of the fold.
    let lift = 1.0 - smoothstep(0.0, 0.08, x - fold);
    return vec4<f32>(before.rgb * (1.0 - 0.12 * lift), 1.0);
}


// Clearing: the old page burns away from where the cursor was. The front
// fingers out raggedly, browning and charring the paper ahead of it, a
// glowing edge, then ash and the fresh page.
fn burnt(px: vec2<f32>, now: vec4<f32>, before: vec4<f32>) -> vec4<f32> {
    let aspect = frame.resolution.x / frame.resolution.y;
    let p = px / frame.resolution * vec2<f32>(aspect, 1.0);
    let seed = vec2<f32>(plan.seed * 1.3, plan.seed * 0.7);
    let lp = px / max(frame.scale, 1.0);
    let ragged = (fbm(p * 3.4 + seed) - 0.5) * 0.4 + (noise(lp * 0.12 + seed) - 0.5) * 0.05;
    let behind = frame.progress * plan.reach - (length(p - vec2<f32>(plan.at_x, plan.at_y)) + ragged);
    let edge = 0.03;
    let ash_rgb = vec3<f32>(0.08, 0.06, 0.05) * (0.7 + 0.6 * noise(lp * 0.3 + seed));
    let flicker = 0.65 + 0.35 * noise(lp * 0.2 + vec2<f32>(frame.time * 6.0, 0.0) + seed);
    if (behind > edge) {
        let ash = 1.0 - smoothstep(edge, edge + 0.14, behind);
        var rgb = mix(now.rgb, ash_rgb, ash * 0.85);
        let speck = step(0.965, hash(floor(lp * 0.5) + seed)) * ash;
        rgb = mix(rgb, vec3<f32>(1.0, 0.48, 0.12), speck * flicker);
        return vec4<f32>(rgb, 1.0);
    }
    if (behind > 0.0) {
        let t = behind / edge;
        let hot = mix(vec3<f32>(1.0, 0.74, 0.3), vec3<f32>(0.6, 0.12, 0.04), t);
        return vec4<f32>(mix(hot * flicker, ash_rgb, smoothstep(0.55, 1.0, t)), 1.0);
    }
    let ahead = -behind;
    var rgb = mix(before.rgb, before.rgb * vec3<f32>(0.66, 0.46, 0.26), (1.0 - smoothstep(0.0, 0.12, ahead)) * 0.8);
    rgb = mix(rgb, vec3<f32>(0.1, 0.06, 0.03), 1.0 - smoothstep(0.0, 0.016, ahead));
    return vec4<f32>(rgb, 1.0);
}

// Closing a pane: its page is torn out of the log the way a page is torn
// from a notebook. The tear runs down a ragged line just inside the pane's
// left edge (the binding), top first; as it goes the page swings away from
// its bottom corner, and once free it falls off to the right, turning. The
// torn edges are pale and fibrous; a ragged stub stays at the binding, and
// fades as the pane goes. The panes taking its place show beneath.
fn torn(px: vec2<f32>, now: vec4<f32>) -> vec4<f32> {
    let res = frame.resolution;
    let aspect = res.x / res.y;
    let area = plan.area;
    let t = frame.progress;
    let p = px / res.y;
    let lo = area.xy * vec2<f32>(aspect, 1.0);
    let hi = (area.xy + area.zw) * vec2<f32>(aspect, 1.0);
    let seed = vec2<f32>(plan.seed, plan.seed * 0.37);
    let span = hi - lo;
    let binding = lo.x + span.x * 0.045;
    // The tear's line at height y: wandering, with fine fibres.
    let tear_x = binding;
    let tear = clamp(t / 0.5, 0.0, 1.0);
    let front = mix(lo.y, hi.y, tear * tear * (3.0 - 2.0 * tear));
    let fly = clamp((t - 0.45) / 0.55, 0.0, 1.0);
    // The page swings about its bottom corner at the binding while it
    // tears, then falls away to the right, turning faster.
    let pivot = vec2<f32>(binding, hi.y);
    let angle = 0.16 * tear + 0.9 * fly * fly;
    let shift = vec2<f32>(span.x * 0.9 * fly * fly, span.y * 0.5 * fly * fly * fly);
    let src = rotate(p - pivot - shift, -angle) + pivot;
    var color = now;
    // The stub at the binding, left of the tear, where it has torn.
    let line_here = tear_x + (fbm(vec2<f32>(p.y * 22.0, 0.0) + seed) - 0.5) * span.x * 0.03
        + (noise(vec2<f32>(p.y * 260.0, 1.0) + seed) - 0.5) * 0.0025;
    let in_pane = all(p >= lo) && all(p <= hi);
    let stub_alpha = 1.0 - smoothstep(0.7, 1.0, t);
    if (in_pane && p.x < line_here) {
        var c = textureLoad(previous, vec2<i32>(clamp(px, vec2<f32>(0.0), res - 1.0)), 0).rgb;
        let edge = 1.0 - smoothstep(0.0, 0.0035, line_here - p.x);
        c = mix(c, vec3<f32>(0.97, 0.94, 0.86), edge * 0.9 * step(p.y, front));
        color = vec4<f32>(mix(now.rgb, c, stub_alpha), 1.0);
    }
    // The page: right of the tear in its own (moving) place.
    let line_src = tear_x + (fbm(vec2<f32>(src.y * 22.0, 0.0) + seed) - 0.5) * span.x * 0.03
        + (noise(vec2<f32>(src.y * 260.0, 1.0) + seed) - 0.5) * 0.0025;
    let page = src.x >= line_src && all(src >= lo) && all(src <= hi);
    // Where it hasn't torn yet the page is still whole: no gap there.
    if (page || (in_pane && p.y > front && fly <= 0.0 && p.x >= line_here)) {
        let at = select(p, src, page);
        var c = textureLoad(previous, vec2<i32>(clamp(at * res.y, vec2<f32>(0.0), res - 1.0)), 0).rgb;
        if (page) {
            let edge = 1.0 - smoothstep(0.0, 0.0035, at.x - line_src);
            c = mix(c, vec3<f32>(0.97, 0.94, 0.86), edge * 0.9 * step(at.y, front + 0.001));
            // Lifting off the sheet, the page catches less light.
            c *= 1.0 - 0.12 * angle;
        }
        color = vec4<f32>(c, 1.0);
    }
    return color;
}
