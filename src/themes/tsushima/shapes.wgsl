// Tsushima's own shapes (mod.rs: DAB, SLASH), for the cursor and for
// struck-out names.
//   dab    a dab of the brush filling the box: ragged edges, pressed wider
//          where it lands at the top, dry streaks as it lifts
//          (params.y = roughness, params.w = seed)
//   slash  a sword cut across the box, rising slightly: tapered to points,
//          ragged, with a hairline scratch beside it (params.y = how far it
//          has been drawn, 0..1, params.w = seed)
//   cut    the long cut dividing two panes, end to end along the box
//          (params.y = how far it has been drawn, params.z = 1 when it runs
//          up and down, params.w = seed)
//   seal   a hanko pressed onto the paper, filling the box (params.y = how
//          far it has come down, 0..1: it prints fully once it lands;
//          params.z = 1 for a failed command's empty, slashed seal;
//          params.w = seed)

fn theme_shape(s: ShapeIn) -> vec4<f32> {
    let kind = u32(s.params.x + 0.5);
    if (kind == 16u) {
        return dab(s);
    }
    if (kind == 17u) {
        return slash(s);
    }
    if (kind == 18u) {
        return cut(s);
    }
    if (kind == 19u) {
        return seal(s);
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

// The cut dividing two panes: one stroke of the blade down (or across) the
// whole box, fast. Sharp at both ends and at the tip while it is still
// moving, swelling and thinning a little along the way, ragged where the
// paper tore, with dry gaps near the ends and a hairline scratch beside it
// over its middle stretch.
fn cut(in: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(in.params.w * 7.31, in.params.w * 3.17);
    let upright = in.params.z > 0.5;
    let along = select(in.local.x, in.local.y, upright);
    let across = select(in.local.y, in.local.x, upright);
    let len = max(select(in.size.x, in.size.y, upright), 1.0);
    let wide = select(in.size.y, in.size.x, upright);
    let s = frame.scale;
    let reached = in.params.y * len;
    if (along > reached) {
        return vec4<f32>(0.0);
    }
    let end = 90.0 * s;
    let ends = min(along, len - along) / end;
    let tip = select(smoothstep(0.0, 28.0 * s, reached - along), 1.0, in.params.y >= 1.0);
    let taper = pow(clamp(ends, 0.0, 1.0), 0.8) * tip;
    // The blade bites in and lifts: thick and thin along the stroke.
    let swell = 0.35 + 1.0 * noise(vec2<f32>(along * 0.006, 2.0) + seed) * noise(vec2<f32>(along * 0.017, 6.0) + seed);
    let thick = 2.0 * s * swell * taper + 0.35 * s * tip;
    // The blade's line drifts gently, and the paper tears raggedly.
    let centre = wide * 0.5 + (noise(vec2<f32>(along * 0.006, 0.0) + seed) - 0.5) * wide * 0.18;
    let ragged = (noise(vec2<f32>(along * 0.45, 5.0) + seed) - 0.5) * thick * 0.5;
    let d = abs(across - centre + ragged) - thick * 0.5;
    var alpha = clamp(0.5 - d, 0.0, 1.0);
    // Dry gaps where the blade skimmed, more towards the ends, and fine
    // scratches through the stroke.
    let dry = smoothstep(0.55, 0.8, noise(vec2<f32>(along * 0.05, 4.0) + seed * 1.3));
    alpha *= 1.0 - 0.75 * dry * (1.0 - 0.5 * clamp(ends * 0.5, 0.0, 1.0));
    alpha *= 1.0 - 0.45 * smoothstep(0.7, 0.9, noise(vec2<f32>(along * 0.9, across * 0.6) + seed * 2.1));

    let t = along / len;
    let hairline = centre + 2.6 * s + (noise(vec2<f32>(along * 0.05, 9.0) + seed) - 0.5) * 1.2 * s;
    let span = smoothstep(0.08, 0.2, t) * (1.0 - smoothstep(0.7, 0.9, t)) * tip;
    let hair = clamp(0.5 - (abs(across - hairline) - 0.35 * s), 0.0, 1.0) * span * 0.55;
    return vec4<f32>(in.color.rgb, in.color.a * max(alpha, hair));
}

// A seal pressed onto the paper: a square of seal ink with softly rounded,
// uneven corners and a border, its characters carved out of it (made up
// from the seed: a few strokes across and down inside the border), the ink
// thin where the paper's tooth missed it. Until it lands it is only a
// shadow of itself. A failed command's seal is empty, a slash through it.
fn seal(in: ShapeIn) -> vec4<f32> {
    let seed = vec2<f32>(in.params.w * 7.31, in.params.w * 3.17);
    let landed = in.params.y >= 1.0;
    let failed = in.params.z > 0.5;
    let size = max(in.size, vec2<f32>(1.0));
    let p = in.local - size * 0.5;
    let half = size * 0.44;
    let r = size.x * 0.1;
    let q = abs(p) - half + r;
    var d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    d += (noise(in.local * 0.35 + seed) - 0.5) * size.x * 0.05;
    var ink = clamp(0.5 - d, 0.0, 1.0);
    let border = size.x * 0.08;
    let inner = d + border * 1.6;
    if (failed) {
        // An empty frame, slashed corner to corner.
        let frame = clamp(0.5 - (abs(d + border * 0.5) - border * 0.5), 0.0, 1.0);
        let along = (p.x + p.y) * 0.7071;
        let slash = clamp(0.5 - (abs((p.x - p.y) * 0.7071) - border * 0.6), 0.0, 1.0)
            * step(abs(along), size.x * 0.4);
        ink = max(frame, slash);
    } else if (inner < 0.0) {
        // Carved strokes, in paper: three across and two down, of lengths
        // made up from the seed, on the seal's inner square.
        let u = (p / (half - border * 1.6) + 1.0) * 0.5;
        let width = 0.11;
        var carved = 0.0;
        for (var k = 0; k < 3; k++) {
            let row = 0.2 + f32(k) * 0.3;
            let first = hash(seed + vec2<f32>(f32(k), 1.0)) * 0.3;
            let last = 1.0 - hash(seed + vec2<f32>(f32(k), 2.0)) * 0.3;
            carved = max(carved, step(abs(u.y - row), width * 0.5) * step(first, u.x) * step(u.x, last));
        }
        for (var k = 0; k < 2; k++) {
            let col = 0.3 + f32(k) * 0.4 + (hash(seed + vec2<f32>(f32(k), 3.0)) - 0.5) * 0.15;
            let first = hash(seed + vec2<f32>(f32(k), 4.0)) * 0.4;
            carved = max(carved, step(abs(u.x - col), width * 0.5) * step(first, u.y));
        }
        ink *= 1.0 - carved;
    }
    // The paper's tooth: specks and patches where the ink did not take.
    let tooth = smoothstep(0.62, 0.8, noise(in.local * 0.6 + seed * 1.9)) * 0.7
        + step(0.93, hash(floor(in.local * 0.8) + seed)) * 0.5;
    ink *= 1.0 - clamp(tooth, 0.0, 0.85);
    let strength = select(0.55, 1.0, landed);
    return vec4<f32>(in.color.rgb, in.color.a * ink * strength);
}
