// Chalkboard's surfaces. Dark: a slate board with years of use on it. Broad
// swirls where the eraser went round, faint ghosts of old writing that never
// quite came off, specks of chalk dust, more dust gathered along the bottom
// where the chalk tray is, and edges a little darker. Light: a slab of white
// stone written on in charcoal: soft cloudy mottling, fine grain lit from
// the top left, tiny pits, a few faint grey veins, and the same smudges and
// ghosts, in charcoal.

// A thin curly line wherever stretched, warped noise crosses its middle:
// reads as a stroke of old handwriting, mostly erased.
fn ghost(p: vec2<f32>, angle: f32, offset: f32) -> f32 {
    let warp = vec2<f32>(noise(p * 0.03 + offset), noise(p * 0.05 + offset + 3.0)) * 12.0;
    let q = rotate(p + warp, angle) * vec2<f32>(0.06, 0.16) + offset;
    let n = noise(q);
    // Only in patches, like words here and there.
    let words = smoothstep(0.6, 0.78, noise(p * 0.006 + offset * 1.7));
    return (1.0 - smoothstep(0.0, 0.04, abs(n - 0.5))) * words;
}

fn surface(px: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    let p = px + board_offset();
    var color = frame.surface.rgb;
    if (look.stone > 0.5) {
        color = stone(p);
    } else {
        // Slate grain: fine and even.
        color += vec3<f32>((tooth(p) - 0.5) * 0.035);
    }
    return use_marks(color, p, uv);
}

// White stone: limestone-like, warm and cool patches, grain and pits.
fn stone(p: vec2<f32>) -> vec3<f32> {
    var color = frame.surface.rgb;
    // Cloudy mottling, a little warmer in some places, cooler in others.
    let cloud = fbm(p * 0.004 + 3.0);
    let tint = fbm(p * 0.0025 + 41.0) - 0.5;
    color *= 1.0 - (cloud - 0.5) * 0.09;
    color += vec3<f32>(0.018, 0.008, -0.012) * tint;
    // Fine grain, lit from the top left so the slab has relief.
    let e = 0.8;
    let lit = (tooth(p - vec2<f32>(e, e)) - tooth(p + vec2<f32>(e, e))) * 0.11;
    color += vec3<f32>(lit + (hash(floor(p * 1.3)) - 0.5) * 0.025);
    // Tiny pits, darker, here and there.
    let cell = floor(p / 5.0);
    if (hash(cell + 0.7) > 0.93) {
        let centre = (cell + vec2<f32>(hash(cell + 1.9), hash(cell + 3.3))) * 5.0;
        let r = 0.5 + 1.1 * hash(cell + 5.1);
        color *= 1.0 - 0.16 * (1.0 - smoothstep(r - 0.5, r + 0.5, distance(p, centre)));
    }
    // A few faint veins: thin grey lines where warped noise crosses its middle.
    let warp = vec2<f32>(fbm(p * 0.003 + 7.0), fbm(p * 0.003 + 13.0)) * 140.0;
    let n = noise(rotate(p + warp, 0.6) * vec2<f32>(0.004, 0.02));
    let vein = (1.0 - smoothstep(0.0, 0.012, abs(n - 0.5))) * smoothstep(0.55, 0.75, fbm(p * 0.002 + 29.0));
    color = mix(color, color * 0.82, vein * 0.6);
    return color;
}

// What use leaves on either surface: eraser smudges, ghosts of old writing,
// specks of dust, dust along the bottom, darker edges.
fn use_marks(base: vec3<f32>, p: vec2<f32>, uv: vec2<f32>) -> vec3<f32> {
    let chalk = frame.ink.rgb;
    var color = base;

    // Eraser swirls: long horizontal sweeps that curve, lighter where the
    // dust was pushed around.
    let w = p + vec2<f32>(fbm(p * 0.002) * 260.0, fbm(p * 0.002 + 5.0) * 120.0);
    let sweep = sin(w.y * 0.045 + sin(w.x * 0.008) * 2.5) * 0.5 + 0.5;
    let reach = smoothstep(0.42, 0.72, fbm(p * vec2<f32>(0.0025, 0.006) + 11.0));
    let smear = (0.5 + 0.5 * sweep) * reach;
    let cloud = smoothstep(0.35, 0.8, fbm(p * 0.004 + 23.0));
    color = mix(color, chalk, clamp((smear * 0.06 + cloud * 0.035) * look.smudges, 0.0, 1.0));

    // Ghosts of old writing.
    var traces = 0.0;
    for (var k = 0; k < 3; k++) {
        traces += ghost(p, -0.08 + f32(k) * 0.05, f32(k) * 31.0);
    }
    color = mix(color, chalk, clamp(traces * 0.035 * look.ghosts, 0.0, 1.0));

    // Specks of dust, and the dust gathered above the chalk tray.
    let speck = step(0.996, hash(floor(p * 0.9))) * hash(floor(p * 0.9) + 3.7);
    let tray = smoothstep(0.86, 1.0, uv.y) * (0.6 + 0.4 * noise(p * vec2<f32>(0.05, 0.4)));
    color = mix(color, chalk, clamp(speck * 0.35 + tray * 0.12 * look.smudges, 0.0, 1.0));

    // A little darker towards the frame.
    let d = distance(uv, vec2<f32>(0.5));
    color *= 1.0 - smoothstep(0.4, 0.9, d) * select(0.35, 0.14, look.stone > 0.5);
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}
