// Chalkboard's own shapes (mod.rs):
//   16 stick   a box rubbed in with the side of a stick of chalk: grainy,
//              its edges uneven (params.y = how far it has been drawn
//              across, 0..1; params.w = seed)
//   17 line    a quick chalk line across the box, a little wavy and rising,
//              drawn in up to params.y of the way (params.w = seed)

fn theme_shape(s: ShapeIn) -> vec4<f32> {
    let kind = u32(s.params.x + 0.5);
    if (kind == 16u) {
        return stick(s);
    }
    if (kind == 17u) {
        return chalk_line(s);
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
