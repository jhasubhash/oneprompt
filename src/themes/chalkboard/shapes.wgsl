// Chalkboard's own shapes (mod.rs):
//   16 stick   a box rubbed in with the side of a stick of chalk: grainy,
//              its edges uneven (params.y = how far it has been drawn
//              across, 0..1; params.w = seed)
//   17 line    a quick chalk line across the box, a little wavy and rising,
//              drawn in up to params.y of the way (params.w = seed)
//   18 rule    the line ruled between two panes, end to end along the box
//              (params.y = how far it has been drawn, params.z = 1 when it
//              runs up and down, params.w = seed)
//   19 stroke  a short chalk stroke corner to corner of the box, down to the
//              right (params.z = 1: up to the right), drawn in up to params.y
//              of the way (params.w = seed)

fn theme_shape(s: ShapeIn) -> vec4<f32> {
    let kind = u32(s.params.x + 0.5);
    if (kind == 16u) {
        return stick(s);
    }
    if (kind == 17u) {
        return chalk_line(s);
    }
    if (kind == 18u) {
        return rule(s);
    }
    if (kind == 19u) {
        return stroke(s);
    }
    return vec4<f32>(0.0);
}

fn grain(p: vec2<f32>, seed: vec2<f32>) -> f32 {
    return noise(p * 0.9 + seed) * 0.6 + hash(floor(p * 1.3) + seed) * 0.4;
}

fn stick(s: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(s.params.w * 5.3, s.params.w * 2.9);
    let p = s.local;
    if (p.x > s.size.x * s.params.y) {
        return vec4<f32>(0.0);
    }
    // Ragged sides.
    let inset = (noise(vec2<f32>(p.y * 0.4, 0.0) + seed) - 0.5) * 2.0;
    let d = max(abs(p.x - s.size.x * 0.5) - s.size.x * 0.5 + 1.0 + inset, abs(p.y - s.size.y * 0.5) - s.size.y * 0.5 + 1.0);
    let inside = clamp(0.5 - d, 0.0, 1.0);
    let catches = smoothstep(0.25, 0.6, grain(p, seed));
    return vec4<f32>(s.color.rgb, s.color.a * inside * mix(0.55, 1.0, catches));
}

fn chalk_line(s: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(s.params.w * 5.3, s.params.w * 2.9);
    let p = s.local;
    let w = s.size.x;
    let h = s.size.y;
    let t = p.x / max(w, 1.0);
    if (t > s.params.y) {
        return vec4<f32>(0.0);
    }
    let rise = mix(0.04, 0.1, hash(seed));
    let wave = (noise(vec2<f32>(p.x * 0.05, 1.0) + seed) - 0.5) * h * 0.12;
    let centre = mix(h * (0.55 + rise), h * (0.55 - rise), t) + wave;
    // Pressed harder in the middle, lifting at the ends.
    let thick = max(h * 0.12, 2.0) * (0.55 + 0.45 * sin(3.14159265 * clamp(t, 0.0, 1.0)));
    let d = abs(p.y - centre) - thick * 0.5;
    let inside = clamp(0.5 - d, 0.0, 1.0);
    let catches = smoothstep(0.22, 0.58, grain(p, seed));
    return vec4<f32>(s.color.rgb, s.color.a * inside * mix(0.35, 1.0, catches));
}

// A line ruled in chalk between two panes: steady but not machine-straight,
// pressed harder in places, broken where the chalk skipped the board's
// tooth, with a faint haze of dust along it. The stick's end is rounder
// while it is still moving.
fn rule(s: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(s.params.w * 5.3, s.params.w * 2.9);
    let upright = s.params.z > 0.5;
    let along = select(s.local.x, s.local.y, upright);
    let across = select(s.local.y, s.local.x, upright);
    let len = max(select(s.size.x, s.size.y, upright), 1.0);
    let wide = select(s.size.y, s.size.x, upright);
    let px = frame.scale;
    let reached = s.params.y * len;
    if (along > reached + 2.0 * px) {
        return vec4<f32>(0.0);
    }
    let centre = wide * 0.5 + (noise(vec2<f32>(along * 0.004, 1.0) + seed) - 0.5) * wide * 0.12;
    let press = 0.7 + 0.5 * noise(vec2<f32>(along * 0.01, 3.0) + seed);
    let thick = 2.2 * px * press;
    var d = abs(across - centre) - thick * 0.5;
    // The rounded end of the stick at the front while it moves.
    if (s.params.y < 1.0 && along > reached - thick) {
        d = max(d, length(vec2<f32>(along - (reached - thick * 0.5), across - centre)) - thick * 0.5);
    }
    let inside = clamp(0.5 - d, 0.0, 1.0);
    let p = vec2<f32>(along, across);
    let catches = smoothstep(0.22, 0.58, grain(p, seed));
    let haze = clamp(1.0 - abs(across - centre) / (wide * 0.45), 0.0, 1.0) * grain(p * 0.7, seed * 1.7) * 0.12;
    return vec4<f32>(s.color.rgb, s.color.a * max(inside * mix(0.35, 1.0, catches), haze));
}

// A short chalk stroke from one corner of the box to the opposite one, as
// in a cross or a tick: pressed harder in the middle, its end rounder while
// the chalk is still moving, broken where it skipped the board's tooth.
fn stroke(s: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(s.params.w * 5.3, s.params.w * 2.9);
    let size = max(s.size, vec2<f32>(1.0));
    var a = vec2<f32>(size.x * 0.1, size.y * 0.1);
    var b = vec2<f32>(size.x * 0.9, size.y * 0.9);
    if (s.params.z > 0.5) {
        a = vec2<f32>(size.x * 0.1, size.y * 0.9);
        b = vec2<f32>(size.x * 0.9, size.y * 0.1);
    }
    let reach = mix(a, b, clamp(s.params.y, 0.0, 1.0));
    let dir = b - a;
    let t = clamp(dot(s.local - a, dir) / max(dot(dir, dir), 1.0), 0.0, clamp(s.params.y, 0.0, 1.0));
    let nearest = a + dir * t;
    let wobble = (noise(vec2<f32>(t * 6.0, 3.0) + seed) - 0.5) * min(size.x, size.y) * 0.05;
    let thick = max(min(size.x, size.y) * 0.13, 2.0) * (0.6 + 0.4 * sin(3.14159265 * t));
    let d = length(s.local - nearest) + wobble - thick * 0.5;
    let inside = clamp(0.5 - d, 0.0, 1.0);
    let catches = smoothstep(0.22, 0.58, grain(s.local, seed));
    return vec4<f32>(s.color.rgb, s.color.a * inside * mix(0.35, 1.0, catches));
}
