// Chalkboard's reveals: an eraser wipes the old look off in a few passes,
// back and forth down the board (or up it), slightly tilted, each pass
// overlapping the last along a wavering line. Behind the eraser the old
// chalk lingers for a moment as streaky smears along its path; at its edge
// dust is pushed ahead of it.

// What mod.rs describes (`PlanData`).
struct Plan {
    passes: f32,  // how many times the eraser crosses the board
    angle: f32,   // tilt of its path, radians
    seed: f32,
    upward: f32,  // 1: from the bottom up (sliding: up)
    mode: f32,    // 0: the eraser; 1: the chalk is blown away; 2: the board slides
    at_x: f32,    // where the chalk starts going from (0..1 of the window)
    at_y: f32,
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
            return slide(px, now_in);
        }
        return blown(px, now_in);
    }
    // The window closing: the board is wiped clean before it goes.
    var now = now_in;
    if (frame.closing > 0.5) {
        now = vec4<f32>(frame.surface.rgb, 1.0);
    }
    let scale = max(frame.scale, 1.0);
    let lp = px / scale + vec2<f32>(plan.seed * 37.0, plan.seed * 11.0);
    let res = frame.resolution;
    let aspect = res.x / res.y;
    let uv = px / res;
    let q = rotate(vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5), plan.angle);
    let x = clamp(q.x / aspect + 0.5, 0.0, 1.0);
    var v = clamp(q.y + 0.5, 0.0, 0.9999);
    if (plan.upward > 0.5) {
        v = 0.9999 - v;
    }

    // Which pass reaches this point, and where along it: the passes
    // alternate direction, and their borders waver so they overlap.
    let n = plan.passes;
    let wobble = (noise(vec2<f32>(lp.x * 0.008, plan.seed * 13.0)) - 0.5) * 0.5;
    let band = clamp(floor(v * n + wobble), 0.0, n - 1.0);
    let forward = fract(band * 0.5) < 0.25;
    let along = select(1.0 - x, x, forward);
    let arrive = (band + along) / n;

    // The eraser's edge is streaky along its path and a little ragged.
    let streaks = noise(lp * vec2<f32>(0.015, 0.35));
    let ragged = (fbm(lp * 0.02 + 5.0) - 0.5) * 0.03 + (streaks - 0.5) * 0.015;
    let behind = frame.progress * 1.04 - (arrive + ragged);
    let shown = smoothstep(0.0, 0.004, behind);

    // Smears of the old chalk just behind the eraser, fading as it moves on.
    var smear = before;
    if (frame.from_color.w < 0.5) {
        let dir = select(-1.0, 1.0, forward) * rotate(vec2<f32>(1.0, 0.0), -plan.angle);
        var sum = vec4<f32>(0.0);
        for (var k = 1; k <= 4; k++) {
            let at = clamp(px - dir * f32(k) * 7.0 * scale, vec2<f32>(0.0), res - 1.0);
            sum += textureLoad(previous, vec2<i32>(at), 0);
        }
        smear = sum / 4.0;
    }
    let linger = (1.0 - smoothstep(0.0, 0.07, behind)) * shown * (0.25 + 0.35 * smoothstep(0.4, 0.8, streaks));
    var color = mix(now, smear, linger * 0.6);

    // Dust pushed along at the eraser's edge.
    let edge = 1.0 - smoothstep(0.0, 0.012, abs(behind));
    let speck = smoothstep(0.55, 0.95, hash(floor(lp * 1.2))) * edge;
    color = mix(mix(before, color, shown), vec4<f32>(frame.ink.rgb, 1.0), speck * 0.4);
    return vec4<f32>(color.rgb, 1.0);
}

// Clearing: the chalk is blown off the board. The marks break up into
// specks of dust that drift away, downwards and out from where the cursor
// was, fading as they go; those nearest it go first. The chalk is what the
// old frame has that the clean board (`scene`) doesn't, so the board itself
// stays put.
fn blown(px: vec2<f32>, now: vec4<f32>) -> vec4<f32> {
    let res = frame.resolution;
    let scale = max(frame.scale, 1.0);
    let t = frame.progress;
    let at = vec2<f32>(plan.at_x, plan.at_y) * res;
    var sum = vec3<f32>(0.0);
    // A few specks may have drifted onto this pixel from where they were.
    for (var k = 0; k < 3; k++) {
        let seed = vec2<f32>(plan.seed + f32(k) * 7.1, f32(k) * 3.3);
        // Where the dust here came from: further back the longer it has
        // been going, its start delayed by the distance from the cursor.
        let away = normalize(px - at + vec2<f32>(0.001, 0.0));
        let delay = clamp(length(px - at) / length(res) * 1.2, 0.0, 0.6);
        let go = clamp((t - delay) / 0.5, 0.0, 1.0);
        let drift = (away * 0.5 + vec2<f32>((noise(px * 0.01 + seed) - 0.5) * 1.2, 1.0)) * go * go * 140.0 * scale;
        let src = clamp(px - drift, vec2<f32>(0.0), res - 1.0);
        let old = textureLoad(previous, vec2<i32>(src), 0).rgb;
        let clean = textureLoad(scene, vec2<i32>(src), 0).rgb;
        // The chalk breaks into specks, fewer and fewer as it goes.
        let speck = step(go * 0.95, hash(floor(src / (1.5 * scale)) + seed));
        sum += (old - clean) * speck * (1.0 - go);
    }
    return vec4<f32>(clamp(now.rgb + sum / 1.5, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// Closing a pane: its board slides up (or down) out of the frame, quickly
// after a heavy start, the panes taking its place behind it; a shadow along
// its edge, where it leaves.
fn slide(px: vec2<f32>, now: vec4<f32>) -> vec4<f32> {
    let res = frame.resolution;
    let area = plan.area;
    let t = frame.progress;
    let move_t = t * t * (3.0 - 2.0 * t) * 0.6 + t * t * 0.4;
    let dy = move_t * area.w * 1.05 * select(1.0, -1.0, plan.upward > 0.5);
    let uv = px / res;
    let src = uv - vec2<f32>(0.0, dy);
    if (src.y >= area.y && src.y <= area.y + area.w) {
        let c = textureLoad(previous, vec2<i32>(clamp(src * res, vec2<f32>(0.0), res - 1.0)), 0).rgb;
        return vec4<f32>(c, 1.0);
    }
    // The edge it leaves from casts a shadow on what is behind.
    let edge = select(area.y + area.w + dy, area.y + dy, plan.upward > 0.5);
    let shadow = 1.0 - smoothstep(0.0, 0.03, abs(uv.y - edge));
    return vec4<f32>(now.rgb * (1.0 - 0.45 * shadow), 1.0);
}
