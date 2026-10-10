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

// Closing a pane: its page is torn out. A ragged tear runs across it (at
// `tilt`), its edges pale and fibrous where the paper parted; the two
// pieces are pulled apart, one up and away, one down and away, and fall,
// the panes taking its place beneath. The old pane is in `previous`.
fn torn(px: vec2<f32>, now: vec4<f32>) -> vec4<f32> {
    let res = frame.resolution;
    let aspect = res.x / res.y;
    let area = plan.area;
    let t = frame.progress;
    let p = px / res.y;
    let centre = (area.xy + area.zw * 0.5) * vec2<f32>(aspect, 1.0);
    let dir = vec2<f32>(cos(plan.tilt), sin(plan.tilt));
    let n = vec2<f32>(-dir.y, dir.x);
    let lo = area.xy * vec2<f32>(aspect, 1.0);
    let hi = (area.xy + area.zw) * vec2<f32>(aspect, 1.0);
    let seed = vec2<f32>(plan.seed, plan.seed * 0.37);
    // How far the tear has run along its line, then the pull apart.
    let run = clamp(t / 0.3, 0.0, 1.0);
    let pull = clamp((t - 0.25) / 0.75, 0.0, 1.0);
    var color = now;
    for (var k = 0; k < 2; k++) {
        let side = select(-1.0, 1.0, k == 1);
        let shift = n * side * pull * 0.06 + dir * side * plan.way * pull * 0.05 + vec2<f32>(0.0, pull * pull * pull * 1.4 * select(0.6, 1.0, k == 1));
        let src = p - shift;
        let r = src - centre;
        let along = dot(r, dir);
        // The tear's ragged line: big wanders and fine fibres.
        let line = (fbm(vec2<f32>(along * 9.0, 0.0) + seed) - 0.5) * 0.05 + (noise(vec2<f32>(along * 90.0, 3.0) + seed) - 0.5) * 0.006;
        let across = dot(r, n) - line;
        let on_side = across * side >= 0.0;
        let in_pane = all(src >= lo) && all(src <= hi);
        if (on_side && in_pane) {
            var c = textureLoad(previous, vec2<i32>(clamp(src * res.y, vec2<f32>(0.0), res - 1.0)), 0).rgb;
            // The torn edge: pale fibres where the paper parted.
            let fibre = 1.0 - smoothstep(0.0, 0.005 + 0.004 * noise(vec2<f32>(along * 140.0, 0.0) + seed), abs(across));
            let opened = max(pull, step(along * plan.way, (run - 0.5) * 3.0));
            c = mix(c, vec3<f32>(0.97, 0.94, 0.86), fibre * 0.85 * opened);
            color = vec4<f32>(c * (1.0 - 0.15 * pull), 1.0);
        }
    }
    return color;
}
