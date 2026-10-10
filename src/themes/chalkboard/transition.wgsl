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
    upward: f32,  // 1: from the bottom up
};

@group(0) @binding(2) var<uniform> plan: Plan;

fn theme_transition(px: vec2<f32>, now: vec4<f32>, before: vec4<f32>) -> vec4<f32> {
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
