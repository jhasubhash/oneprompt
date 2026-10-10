// Logbook's transitions: a page turning. The fold sweeps across the window
// (right to left, or back), slightly tilted. Beyond it, still to turn, is
// the old page; behind it the new one shows, shaded where the turning page
// still shadows it; at the fold the page's underside curls over, lit along
// its ridge. Closing the window turns to the inside of the cover.

// What mod.rs describes (`PlanData`).
struct Plan {
    way: f32,   // 1: turning right to left, -1: left to right
    tilt: f32,  // of the fold, radians
    seed: f32,
    _pad: f32,
};

@group(0) @binding(2) var<uniform> plan: Plan;

fn theme_transition(px: vec2<f32>, now_in: vec4<f32>, before: vec4<f32>) -> vec4<f32> {
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

