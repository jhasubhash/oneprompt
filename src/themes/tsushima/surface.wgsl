// Tsushima's sheet: hand-made washi, procedural, so it is resolution
// independent and costs no image assets.

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var k = 0; k < 5; k++) {
        v += amp * noise(q);
        q = q * 2.03 + vec2<f32>(17.0, 9.0);
        amp *= 0.5;
    }
    return v;
}

fn vignette(uv: vec2<f32>) -> f32 {
    let d = distance(uv, vec2<f32>(0.5));
    return smoothstep(0.35, 0.85, d);
}

// Hand-made washi: a rough, lumpy tooth lit from the top left, mottled pulp,
// long fibers, and the hard-edged tide lines water leaves when it dries.
fn washi(px_in: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    // The session seed shifts every pattern, so each launch is a new sheet.
    let px = px_in + sheet_offset();

    // Dark sheets (konshi) are shaded towards a lighter tone, so their
    // vignette has to darken separately instead of going through `shade`.
    let dark_sheet = dot(frame.surface.rgb, vec3<f32>(0.3, 0.59, 0.11)) < 0.5;
    let clouds = fbm(px * 0.006);
    let mottle = noise(px * 0.05) * 0.6 + noise(px * 0.13) * 0.4;
    let grain = hash(px) - 0.5;
    // `texture` scales the paper itself; `stains` the water marks below.
    let texture = look.shade.w;
    let stains = look.stains;
    var shade = (clouds * 0.3 + mottle * 0.12 + grain * 0.08) * texture;
    if (!dark_sheet) {
        // Parchment browns heavily towards the corners, as in the paintings.
        shade += vignette(uv) * 0.75;
    }

    // Water stains: regions of a slow noise field. Pigment collects at the
    // edge where the water stopped: a thin, dark, slightly ragged tide line
    // with a faint wash inside.
    //
    // Text starts on the left, so the stains keep mostly to the right: the
    // field is tilted to rise across the sheet (tide lines are where it
    // crosses a level), and what still reaches the left half fades out.
    let tilt = (uv.x - 0.5) * 0.12;
    let field = fbm(px * 0.0016 + vec2<f32>(3.7, 1.3)) + tilt + (noise(px * 0.08) - 0.5) * 0.012;
    let keep = mix(0.2, 1.0, smoothstep(0.2, 0.62, uv.x));
    for (var k = 0; k < 3; k++) {
        let level = 0.58 + f32(k) * 0.045;
        let inside = field - level;
        let edge = smoothstep(-0.002, 0.0, inside);
        shade += edge * (0.06 + 0.5 * exp(-max(inside, 0.0) * 260.0)) * stains * keep;
    }

    var color = mix(frame.surface.rgb, look.shade.rgb, clamp(shade, 0.0, 1.0));

    // Relief: light the tooth from the top left so the sheet reads as rough.
    let e = 0.75;
    let dx = tooth(px + vec2<f32>(e, 0.0)) - tooth(px - vec2<f32>(e, 0.0));
    let dy = tooth(px + vec2<f32>(0.0, e)) - tooth(px - vec2<f32>(0.0, e));
    let lit = clamp(-(dx + dy) * 1.6, -1.0, 1.0);
    color = color * (1.0 + lit * select(0.11, 0.55, dark_sheet) * texture);
    if (dark_sheet) {
        color = color * (1.0 - vignette(uv) * 0.35);
    }

    // Fibers: mostly lighter than the sheet, a few darker ones.
    var light = 0.0;
    for (var k = 0; k < 3; k++) {
        light += fiber(px, 0.5 + f32(k) * 1.9, f32(k) * 21.0);
    }
    color = mix(color, mix(color, vec3<f32>(1.0), select(0.1, 0.07, dark_sheet)), clamp(light * texture, 0.0, 1.0));
    let dark = fiber(px, 2.4, 57.0);
    color = mix(color, look.shade.rgb * 0.8, clamp(dark * select(0.2, 0.18, dark_sheet) * texture, 0.0, 1.0));

    // A few flicks of ink splatter in the top-right corner, well away from
    // where the text starts.
    let corner = smoothstep(0.55, 1.0, uv.x) * smoothstep(0.45, 0.0, uv.y);
    let cell = floor(px / 9.0);
    let h = hash(cell + 0.5);
    if (h > 1.0 - 0.06 * corner * look.splatter.w) {
        let center = (cell + vec2<f32>(hash(cell + 1.3), hash(cell + 2.9))) * 9.0;
        let radius = 0.6 + 2.4 * hash(cell + 4.1) * hash(cell + 7.7);
        let dot_mask = 1.0 - smoothstep(radius - 0.6, radius + 0.6, distance(px, center));
        color = mix(color, look.splatter.rgb, dot_mask * 0.8);
    }
    return clamp(worn(color, px, uv), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Use wears the sheet (`look.wear`, 0..1): the washi yellows, more at its
// edges, and creases from being folded show up: a fold or two across the
// sheet, a dark valley beside a light ridge, crumpled small creases around
// them, each line appearing as the wear passes its own point.
fn worn(color_in: vec3<f32>, px: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    let wear = look.wear;
    if (wear <= 0.0) {
        return color_in;
    }
    let dark_sheet = dot(frame.surface.rgb, vec3<f32>(0.3, 0.59, 0.11)) < 0.5;
    var color = color_in;
    // Yellowing: towards old gold on the light sheet, a browner black on
    // the dark one; faster at the edges, where hands hold it.
    let edges = 0.6 + 0.4 * vignette(uv);
    let aged = select(vec3<f32>(0.78, 0.6, 0.32), vec3<f32>(0.09, 0.06, 0.035), dark_sheet);
    color = mix(color, color * aged / max(max(frame.surface.r, frame.surface.g), 0.2), wear * 0.35 * edges);
    // Folds: a long one across and one down, at places of the sheet's own;
    // then smaller creases, each from its own point in the wear.
    let s = vec2<f32>(fract(frame.seed * 0.371), fract(frame.seed * 0.613));
    var crease = 0.0;
    for (var k = 0; k < 6; k++) {
        let fk = f32(k);
        let due = select(0.15 + fk * 0.13, fk * 0.25, k < 2);
        let strength = smoothstep(due, due + 0.2, wear);
        if (strength <= 0.0) {
            continue;
        }
        let origin = vec2<f32>(fract(s.x + fk * 0.618), fract(s.y + fk * 0.382));
        let angle = select(hash(vec2<f32>(fk, 3.0)) * 3.1416, select(1.5708, 0.0, k == 0) + (s.x - 0.5) * 0.3, k < 2);
        let dir = vec2<f32>(cos(angle), sin(angle));
        let q = uv * vec2<f32>(frame.resolution.x / frame.resolution.y, 1.0) - origin * vec2<f32>(frame.resolution.x / frame.resolution.y, 1.0);
        let along = dot(q, dir);
        let reach = select(0.35 + 0.3 * hash(vec2<f32>(fk, 9.0)), 2.0, k < 2);
        let wobble = (noise(vec2<f32>(along * 6.0, fk * 7.0)) - 0.5) * 0.02;
        let across = dot(q, vec2<f32>(-dir.y, dir.x)) + wobble;
        let fade = 1.0 - smoothstep(reach * 0.6, reach, abs(along));
        // A valley in shadow on one side, a ridge catching the light on the other.
        let w = 0.0035;
        let valley = exp(-pow((across + w) / w, 2.0));
        let ridge = exp(-pow((across - w) / w, 2.0));
        crease += (ridge * 0.7 - valley) * strength * fade * select(1.0, 0.55, k >= 2);
    }
    let light = select(vec3<f32>(1.0, 0.97, 0.9), frame.ink.rgb, dark_sheet);
    color = mix(color, select(color * 0.72, light, crease > 0.0), clamp(abs(crease), 0.0, 1.0) * 0.45);
    return color;
}

fn surface(px: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    return washi(px, uv);
}
