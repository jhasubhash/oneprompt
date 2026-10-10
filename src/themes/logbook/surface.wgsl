// Logbook's page. By day: cream ledger paper, laid lines faint across it
// (the wire marks of hand-made paper), a ruled line under every row of text
// in faded blue, a red double margin left of the text, brown foxing spots,
// edges toned darker, and a compass rose drawn faintly in the bottom-right
// corner. By night the same page by lamplight: brown, the lamp's warm pool
// in the middle of it, dark at the edges, the rules and rose in a faint
// amber.

fn surface(px: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    let scale = max(frame.scale, 1.0);
    let p = px / scale + page_offset();
    let night = look.night > 0.5;
    var color = frame.surface.rgb;

    // The pulp: soft clouds, and laid lines from the mould's wires.
    let cloud = fbm(p * 0.006);
    color *= 1.0 - (cloud - 0.5) * select(0.08, 0.16, night);
    let laid = sin(p.y * 0.9 + noise(p * 0.02) * 2.0) * 0.5 + 0.5;
    color *= 1.0 - laid * 0.012;
    color *= 1.0 + (hash(floor(p * 1.2)) - 0.5) * 0.02;

    // Foxing: brown spots of age, a few large and faded, more small ones.
    let fox_rgb = select(vec3<f32>(0.6, 0.42, 0.24), vec3<f32>(0.05, 0.03, 0.02), night);
    for (var k = 0; k < 2; k++) {
        let size = select(70.0, 26.0, k == 1);
        let cell = floor(p / size + f32(k) * 7.3);
        let h = hash(cell + 3.1);
        if (h > 1.0 - 0.18 * look.foxing) {
            let centre = (cell + vec2<f32>(hash(cell + 0.4), hash(cell + 9.2)) - f32(k) * 7.3) * size;
            let r = size * (0.08 + 0.18 * hash(cell + 5.5));
            let ragged = (noise(p * 0.3 + cell) - 0.5) * r * 0.5;
            let d = distance(p, centre) + ragged;
            let spot = (1.0 - smoothstep(r * 0.5, r, d)) * (0.35 + 0.65 * hash(cell + 2.2));
            color = mix(color, color * fox_rgb / max(frame.surface.r, 0.2), spot * 0.22 * min(look.foxing, 1.5));
        }
    }

    // The rules: one under each row of text, on the grid's lines, and the
    // red double margin to the left of the text.
    let rule_rgb = select(vec3<f32>(0.42, 0.52, 0.66), vec3<f32>(0.62, 0.5, 0.32), night);
    if (look.rules > 0.0 && frame.cell.y > 1.0) {
        let row = (px.y - frame.grid_origin.y) / frame.cell.y;
        if (row > 0.6) {
            let below = (fract(row) - 0.0) * frame.cell.y;
            let line = 1.0 - smoothstep(0.0, scale * 0.9, abs(below - scale * 0.5));
            let wobble = 0.85 + 0.15 * noise(vec2<f32>(px.x * 0.01, floor(row)));
            color = mix(color, rule_rgb, line * 0.35 * look.rules * wobble);
        }
        let margin = frame.grid_origin.x - 10.0 * scale;
        let red = select(vec3<f32>(0.66, 0.24, 0.2), vec3<f32>(0.62, 0.3, 0.2), night);
        for (var k = 0; k < 2; k++) {
            let at = margin - f32(k) * 3.0 * scale;
            let m = 1.0 - smoothstep(0.0, scale * 0.8, abs(px.x - at));
            color = mix(color, red, m * 0.5 * look.rules);
        }
    }

    // The compass rose, drawn faintly in the bottom-right corner: rings and
    // sixteen points, the four cardinal ones longest.
    if (look.rose > 0.0) {
        let res = frame.resolution / scale;
        let r0 = min(res.x, res.y) * 0.17;
        let centre = res - vec2<f32>(r0 * 1.35, r0 * 1.25);
        let q = px / scale - centre;
        let r = length(q);
        let a = atan2(q.y, q.x);
        let ring = (1.0 - smoothstep(0.0, 0.9, abs(r - r0))) + (1.0 - smoothstep(0.0, 0.7, abs(r - r0 * 0.9))) * 0.7;
        let sector = a / 6.2831853 * 16.0;
        let k = floor(sector + 0.5);
        let off = abs(fract(sector + 0.5) - 0.5);
        let long_point = select(select(0.55, 0.75, (i32(k) % 2) == 0), 1.15, (i32(k) % 4) == 0);
        let point = (1.0 - smoothstep(0.0, 0.06, off * (1.0 + r / r0 * 3.0) - (1.0 - r / (r0 * long_point)) * 0.04))
            * step(r, r0 * long_point);
        let rose_rgb = select(vec3<f32>(0.42, 0.34, 0.26), vec3<f32>(0.7, 0.56, 0.36), night);
        let fade = 0.12 * look.rose * (0.7 + 0.3 * noise(q * 0.2));
        color = mix(color, rose_rgb, clamp((ring + point * 0.8) * fade, 0.0, 1.0));
    }

    // Toned edges by day; by night the lamp's pool of light.
    let d = distance(uv, vec2<f32>(0.5, 0.55));
    if (night) {
        let lamp = 1.0 - smoothstep(0.1, 0.75, d);
        color = color * (0.55 + 0.75 * lamp) + vec3<f32>(0.06, 0.035, 0.0) * lamp;
    } else {
        color *= 1.0 - smoothstep(0.35, 0.85, d) * 0.22;
    }

    // Use: the page yellows and its edges darken with handling.
    let wear = look.wear;
    if (wear > 0.0) {
        let edge = smoothstep(0.3, 0.8, d);
        color = mix(color, color * select(vec3<f32>(0.9, 0.78, 0.55), vec3<f32>(0.8, 0.65, 0.45), night), wear * (0.3 + 0.5 * edge));
    }
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}
