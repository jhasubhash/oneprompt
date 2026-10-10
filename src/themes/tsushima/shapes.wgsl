// Tsushima's own shapes (mod.rs: DAB, SLASH), for the cursor and for
// struck-out names.
//   dab    a dab of the brush filling the box: ragged edges, pressed wider
//          where it lands at the top, dry streaks as it lifts
//          (params.y = roughness, params.w = seed)
//   slash  a sword cut across the box, rising slightly: tapered to points,
//          ragged, with a hairline scratch beside it (params.y = how far it
//          has been drawn, 0..1, params.w = seed)

fn theme_shape(s: ShapeIn) -> vec4<f32> {
    let kind = u32(s.params.x + 0.5);
    if (kind == 16u) {
        return dab(s);
    }
    if (kind == 17u) {
        return slash(s);
    }
    return vec4<f32>(0.0);
}

// A dab of the brush: a rounded box whose sides wobble, a little wider at the
// top where the brush presses down and narrower as it lifts at the bottom,
// with dry streaks running down it that open up towards the end.
fn dab(in: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(in.params.w * 7.31, in.params.w * 3.17);
    let rough = in.params.y;
    let size = in.size - 2.0; // the box has a pixel of room for the ragged edge
    let p = in.local - in.size * 0.5;
    let down = clamp(in.local.y / in.size.y, 0.0, 1.0);
    let half = vec2<f32>(size.x * 0.5 * mix(1.06, 0.86, down), size.y * 0.5);
    let r = min(size.x, size.y) * 0.28;
    // Sides: the left and right edges wander along the stroke; the top and
    // bottom along the width.
    let side = (noise(vec2<f32>(p.y * 0.35, 0.0) + seed) - 0.5) * 2.0;
    let ends = (noise(vec2<f32>(0.0, p.x * 0.5) + seed * 1.7) - 0.5) * 2.0;
    let q = abs(p) - half + r;
    var d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    d += rough * (abs(side) * 1.2 * select(0.3, 1.0, q.x > q.y) + abs(ends) * 0.9 * select(0.3, 1.0, q.y >= q.x));
    d += rough * (hash(floor(in.local) + seed) - 0.5) * 0.6;
    var alpha = clamp(0.5 - d, 0.0, 1.0);
    // Dry streaks along the stroke, more of them as the brush runs dry.
    let streak = noise(vec2<f32>(p.x * 0.9, p.y * 0.07) + seed * 2.3);
    alpha *= 1.0 - rough * 0.55 * smoothstep(0.58, 0.88, streak) * mix(0.25, 1.0, down);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}

// A sword cut: one quick stroke across the box, entering low on the left and
// leaving higher on the right with a slight bow. Thickest mid-way, tapering
// to sharp points at both ends and at the blade's tip while it is still
// cutting; ragged where the paper tore, with dry gaps near the ends, and a
// hairline scratch running beside it where the blade's edge grazed.
fn slash(in: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(in.params.w * 7.31, in.params.w * 3.17);
    let progress = in.params.y;
    let w = in.size.x;
    let h = in.size.y;
    let p = in.local;
    let t = p.x / max(w, 1.0);
    if (t > progress) {
        return vec4<f32>(0.0);
    }
    let rise = mix(0.1, 0.17, hash(seed));
    let centre = mix(h * (0.56 + rise), h * (0.56 - rise), t) - sin(3.14159265 * t) * h * 0.04;
    // Sharp at both ends, and at the tip while the blade is still moving.
    let taper = pow(sin(3.14159265 * t), 0.6) * smoothstep(0.0, 0.1, progress - t + select(0.0, 0.1, progress >= 1.0));
    let thick = max(h * 0.14, 2.2) * taper;
    let ragged = (noise(vec2<f32>(p.x * 0.45, 0.0) + seed) - 0.5) * thick * 0.5;
    let d = abs(p.y - centre + ragged) - thick * 0.5;
    var alpha = clamp(0.5 - d, 0.0, 1.0);
    let dry = smoothstep(0.62, 0.85, noise(vec2<f32>(p.x * 0.3, 4.0) + seed * 1.3));
    alpha *= 1.0 - 0.6 * dry * (1.0 - taper);

    // The hairline, a little below, over the middle stretch.
    let line = centre + h * 0.13 + (noise(vec2<f32>(p.x * 0.2, 9.0) + seed) - 0.5) * 1.5;
    let span = smoothstep(0.12, 0.3, t) * (1.0 - smoothstep(0.62, 0.82, t));
    let hair = clamp(0.5 - (abs(p.y - line) - 0.5), 0.0, 1.0) * span * 0.7;
    return vec4<f32>(in.color.rgb, in.color.a * max(alpha, hair));
}
