// Logbook's own shapes (mod.rs):
//   16 quill  a stroke of the steel nib across the box (params.z = 0: along
//             the middle, rising slightly; 1: corner to corner, down to the
//             right; 2: up to the right), drawn in up to params.y of the way:
//             thin, swelling under pressure, a hairline as it lifts
//             (params.w = seed)
//   17 blot   a blot of ink spreading into the paper, filling the box's
//             circle (params.y = how far it has spread, 0..1)
//   18 rule   the red double line ruled between panes, end to end along the
//             box (params.y = how far ruled, params.z = 1 up and down)
//   19 seal   a seal pressed into red wax: a drop with a raised rim and an
//             anchor (params.z = 1: the seal broken in two), printing once
//             params.y reaches 1

fn theme_shape(s: ShapeIn) -> vec4<f32> {
    let kind = u32(s.params.x + 0.5);
    if (kind == 16u) {
        return quill(s);
    }
    if (kind == 17u) {
        return blot(s);
    }
    if (kind == 18u) {
        return rule(s);
    }
    if (kind == 19u) {
        return seal(s);
    }
    return vec4<f32>(0.0);
}

// Distance from `p` to the segment a..b, and how far along it, 0..1.
fn segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let d = b - a;
    let t = clamp(dot(p - a, d) / max(dot(d, d), 1e-4), 0.0, 1.0);
    return vec2<f32>(length(p - (a + d * t)), t);
}

fn quill(s: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(s.params.w * 4.1, s.params.w * 2.7);
    let size = max(s.size, vec2<f32>(1.0));
    var a = vec2<f32>(0.0, size.y * 0.6);
    var b = vec2<f32>(size.x, size.y * 0.5);
    if (s.params.z > 1.5) {
        a = vec2<f32>(size.x * 0.12, size.y * 0.88);
        b = vec2<f32>(size.x * 0.88, size.y * 0.12);
    } else if (s.params.z > 0.5) {
        a = vec2<f32>(size.x * 0.12, size.y * 0.12);
        b = vec2<f32>(size.x * 0.88, size.y * 0.88);
    }
    let hit = segment(s.local, a, b);
    if (hit.y > s.params.y) {
        return vec4<f32>(0.0);
    }
    // A steel nib: thin, swelling where it presses, lifting to a hairline.
    let press = 0.55 + 0.45 * sin(3.14159265 * hit.y) * (0.8 + 0.4 * noise(vec2<f32>(hit.y * 5.0, 0.0) + seed));
    let thick = max(frame.scale * 1.6, 1.2) * press;
    let ink = clamp(0.5 - (hit.x - thick * 0.5), 0.0, 1.0);
    return vec4<f32>(s.color.rgb, s.color.a * ink);
}

fn blot(s: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(s.params.w * 3.3, s.params.w * 1.9);
    let c = s.size * 0.5;
    let p = s.local - c;
    let r = min(c.x, c.y) * (0.45 + 0.55 * s.params.y);
    let a = atan2(p.y, p.x);
    let edge = r * (0.85 + 0.25 * noise(vec2<f32>(a * 2.0, 0.0) + seed));
    let d = length(p) - edge;
    // Darker at its rim, where the ink dried first.
    let inside = clamp(0.5 - d, 0.0, 1.0);
    let rim = 1.0 - smoothstep(0.0, r * 0.35, -d);
    return vec4<f32>(s.color.rgb, s.color.a * inside * (0.65 + 0.35 * rim));
}

fn rule(s: ShapeIn) -> vec4<f32> {
    let upright = s.params.z > 0.5;
    let along = select(s.local.x, s.local.y, upright);
    let across = select(s.local.y, s.local.x, upright);
    let len = max(select(s.size.x, s.size.y, upright), 1.0);
    let wide = select(s.size.y, s.size.x, upright);
    if (along > s.params.y * len) {
        return vec4<f32>(0.0);
    }
    let gap = 1.6 * frame.scale;
    let w = 0.6 * frame.scale;
    let one = 1.0 - smoothstep(0.0, w, abs(across - wide * 0.5 + gap));
    let two = 1.0 - smoothstep(0.0, w, abs(across - wide * 0.5 - gap));
    return vec4<f32>(s.color.rgb, s.color.a * max(one, two));
}

fn seal(s: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(s.params.w * 5.7, s.params.w * 2.1);
    let landed = s.params.y >= 1.0;
    let size = max(s.size, vec2<f32>(1.0));
    let p = (s.local - size * 0.5) / (size.x * 0.5);
    let r = length(p);
    let a = atan2(p.y, p.x);
    // The drop of wax: a blob, wider where it ran.
    let edge = 0.86 + 0.08 * noise(vec2<f32>(a * 1.5, 0.0) + seed) + 0.05 * sin(a * 5.0 + seed.x);
    var wax = clamp((edge - r) * size.x * 0.3, 0.0, 1.0);
    if (!landed) {
        return vec4<f32>(s.color.rgb, s.color.a * wax);
    }
    // Lit from the top left: the raised rim catches the light, the pressed
    // middle is a shade darker.
    let rim = smoothstep(0.5, 0.62, r) * (1.0 - smoothstep(0.7, 0.8, r));
    let light = clamp(-dot(normalize(p + vec2<f32>(1e-4)), vec2<f32>(0.7, 0.7)), -1.0, 1.0);
    var rgb = s.color.rgb * (0.85 + 0.25 * rim * light);
    // The impression: an anchor, or a crack across a broken seal.
    var mark = 0.0;
    if (s.params.z > 0.5) {
        let crack = abs(p.y - p.x * 0.4 + (noise(p * 6.0 + seed) - 0.5) * 0.12);
        mark = 1.0 - smoothstep(0.02, 0.06, crack);
        wax *= 1.0 - mark * 0.9;
    } else {
        let shank = (1.0 - smoothstep(0.03, 0.06, abs(p.x))) * step(-0.38, p.y) * step(p.y, 0.3);
        let stock = (1.0 - smoothstep(0.03, 0.06, abs(p.y + 0.24))) * step(abs(p.x), 0.18);
        let ring = 1.0 - smoothstep(0.02, 0.05, abs(length(p - vec2<f32>(0.0, -0.44)) - 0.07));
        let arms = (1.0 - smoothstep(0.03, 0.06, abs(length(p - vec2<f32>(0.0, 0.0)) - 0.3))) * step(0.12, p.y);
        mark = max(max(shank, stock), max(ring, arms));
        rgb = rgb * (1.0 - 0.35 * mark);
    }
    return vec4<f32>(rgb, s.color.a * wax);
}
