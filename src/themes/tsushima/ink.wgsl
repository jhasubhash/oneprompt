// Tsushima's ink: how the ink layer lands on the washi, the way sumi does:
//   bleed  - ink wicks past strokes in patches, further along the fibers;
//            stroke edges are ragged, so no two letters look the same
//   nib    - glyphs are swept with a slanted brush, so strokes across it come
//            out thick and strokes along it thin, with pressure varying
//   grain  - dry-brush streaks: the core of a stroke is not perfectly solid
//   edge   - pigment pools where a stroke dries, darkening its rim
//   write  - new characters are written by hand (`written`, from the
//            framework's ink stage), applied to every sample
//   paper  - the ink takes on the sheet's own grain: the brush skips the
//            tooth's peaks, ink settles in its hollows, fibers lie over it
//            (the same tooth and fibers surface.wgsl draws, at the same
//            places; common.wgsl)
//   creep  - the bleed keeps wicking outwards for a while after a character
//            is written, while the pen moves on; every character carries its
//            own load of ink, so some bleed more than others.
//   drying - fresh ink is wet (`wetness`): heavier, with a glossy sheen that
//            dulls as it dries to sumi's matte black
//   age    - ink an hour old (`aged`) has faded a little and browned, as old
//            sumi does on washi

const TAPS: i32 = 12;
const NIB_TAPS: i32 = 3;
// The brush is held tilted up to the right, as for most calligraphy.
const NIB_ANGLE: f32 = -0.62;
const NIB_DIR: vec2<f32> = vec2<f32>(0.8139, -0.5810); // (cos, sin) of NIB_ANGLE

struct Paper {
    peak: f32,   // on a raised point of the tooth, 0..1
    hollow: f32, // in a low spot, 0..1
    fibers: f32, // under one of the light fibers, 0..1
};

fn paper_at(px: vec2<f32>) -> Paper {
    let lp = px / max(frame.scale, 1.0) + sheet_offset();
    let h = tooth(lp);
    var light = 0.0;
    for (var k = 0; k < 3; k++) {
        light += fiber(lp, 0.5 + f32(k) * 1.9, f32(k) * 21.0);
    }
    return Paper(smoothstep(0.52, 0.78, h), 1.0 - smoothstep(0.22, 0.46, h), clamp(light, 0.0, 1.0));
}

struct Bleed {
    growth: f32, // how far the bleed has crept, 0..1 of its reach
    scale: f32,  // this character's load of ink, around 1
};

// The bleed of the character whose cell holds `p`. Ink wicks into paper
// like water: quickly at first, then slower and slower (distance grows
// with the square root of time), so it is still creeping well after the
// pen has moved on to the next letters.
fn bleed_of(p: vec2<f32>) -> Bleed {
    let g = (p - frame.grid_origin) / frame.cell;
    if (any(g < vec2<f32>(0.0)) || any(g >= frame.grid_size)) {
        return Bleed(1.0, 1.0);
    }
    let cell = floor(g);
    let birth = textureLoad(births, vec2<i32>(cell), 0).r;
    // The birth time is part of the hash, so retyping a cell gives it a new load.
    let h = hash(cell + vec2<f32>(fract(birth * 0.731) * 61.0, fract(frame.seed * 0.173) * 37.0));
    let scale = 1.0 + look.bleed_variation * (h * 2.0 - 1.0);
    var growth = 1.0;
    if (frame.write_time > 0.0 && look.bleed_time > 0.0) {
        growth = 0.12 + 0.88 * sqrt(clamp((frame.time - birth) / look.bleed_time, 0.0, 1.0));
    }
    return Bleed(growth, scale);
}

fn ink_at(pos: vec2<f32>, sp: vec2<f32>) -> vec4<f32> {
    return sample_at(pos) * written(pos, sp);
}

fn theme_ink(px: vec2<f32>) -> vec4<f32> {
    if (look.bleed <= 0.0 && look.grain <= 0.0 && look.nib <= 0.0) {
        return sample_at(px);
    }
    let sp = px + sheet_offset();

    // Ragged edges: look the ink up through a small wobble, so stroke
    // outlines are uneven instead of the font's perfect curves.
    let wobble = (vec2<f32>(noise(sp * 0.45), noise(sp * 0.45 + 17.0)) - 0.5) * look.bleed * 0.9;

    // Brush nib: sweep the glyph along a slanted segment and keep the
    // densest ink. Pressure (the sweep length) swells and fades slowly
    // across the page, so stroke weight is never quite uniform.
    let pressure = 0.6 + 0.6 * noise(sp * 0.025 + 11.0);
    let sweep = NIB_DIR * look.nib * pressure * 0.5;
    var core = ink_at(px + wobble, sp);
    for (var j = 1; j <= NIB_TAPS; j++) {
        let t = f32(j) / f32(NIB_TAPS);
        let a = ink_at(px + wobble + sweep * t, sp);
        let b = ink_at(px + wobble - sweep * t, sp);
        if (a.a > core.a) { core = a; }
        if (b.a > core.a) { core = b; }
    }

    // How much this spot bleeds: some patches barely, some a lot.
    let amount = smoothstep(0.3, 0.85, noise(sp * 0.035) * 0.7 + noise(sp * 0.11) * 0.3);
    // Ink travels along the fibers, so the spread is stretched in one
    // direction that slowly changes across the sheet.
    let fiber_angle = noise(sp * 0.008) * 6.2831853;

    var halo = vec4<f32>(0.0);
    if (look.bleed > 0.0) {
        let jitter = hash(px) * 6.2831853;
        let reach = look.bleed * (0.25 + 1.75 * amount);
        for (var k = 0; k < TAPS; k++) {
            let angle = f32(k) * 6.2831853 / f32(TAPS) + jitter;
            let r = reach * (0.3 + 0.7 * hash(px + f32(k) * 13.7));
            let offset = rotate(vec2<f32>(cos(angle) * 1.7, sin(angle) * 0.55) * r, fiber_angle);
            // Each tap reaches only as far as the bleed of the character it
            // lands on has crept, scaled by that character's ink load; fresh
            // bleed is fainter too.
            let b = bleed_of(px + offset);
            halo += ink_at(px + offset * b.scale * b.growth, sp) * mix(0.35, 1.0, b.growth);
        }
        halo /= f32(TAPS);
    }

    // Capillary feathering: the halo breaks up into fine tendrils instead
    // of a smooth glow.
    let capillary = smoothstep(0.35, 0.75, noise(rotate(sp, fiber_angle) * vec2<f32>(0.25, 1.1)));
    // The sheet under the ink: the brush skips the tooth's peaks (dry
    // flecks in the letters, in the paper's own pattern), ink settles
    // heavier in the hollows, and light fibers lie over it. Only worked out
    // where there is ink.
    let bite = look.absorb * min(look.shade.w, 1.5);
    var sheet = Paper(0.0, 0.0, 0.0);
    if (bite > 0.0 && core.a + halo.a > 0.002) {
        sheet = paper_at(px);
    }

    // Bleed runs along the hollows and stops at the peaks.
    let wick = halo.a * mix(0.15, 1.0, capillary) * (0.35 + 0.65 * amount)
        * (1.0 + bite * (0.6 * sheet.hollow - 0.5 * sheet.peak));

    // Ink soaks in: antialiased edges of thin strokes saturate towards solid;
    // the load of ink varies a little, and dry-brush grain thins it in places.
    // Dry-brush streaks run with the brush: fine across it, long along it.
    let streak = noise(rotate(sp, -NIB_ANGLE) * vec2<f32>(0.15, 1.3) + 3.1);
    let grain = 1.0 - look.grain * smoothstep(0.4, 0.95, streak * 0.75 + hash(px) * 0.25);
    let load = 0.9 + 0.1 * noise(sp * 0.02 + 5.0);
    let tooth_skip = 1.0 - bite * (0.42 * sheet.peak + 0.3 * sheet.fibers);
    let settle = 1.0 + bite * 0.25 * sheet.hollow;
    let body = clamp(core.a * 1.3 * settle, 0.0, 1.0) * grain * load * tooth_skip;

    // After the brush front (applied per sample above), the wet ink settles
    // over a few write times. (The bleed's creep is in the halo taps.)
    let age = cell_ink(px, vec2<f32>(0.0)).age;
    var wet = 0.0;
    if (age < frame.write_time * 4.0) {
        wet = 1.0 - clamp(age / (frame.write_time * 4.0), 0.0, 1.0);
    }
    let soaked = wetness(px);
    wet = max(wet, soaked);

    // Pigment pools at the rim: strongest where the halo is half covered.
    let rim = clamp(halo.a * (1.0 - halo.a) * 4.0, 0.0, 1.0) * look.edge;

    // Wet ink is a touch heavier.
    let alpha = clamp(
        max(body * (1.0 + 0.25 * wet), wick * (0.85 + 0.3 * soaked)) + rim * 0.2 * (1.0 - body),
        0.0,
        1.0,
    );
    // Colour: the stroke's own colour, or the spread colour in the halo.
    var rgb = halo.rgb / max(halo.a, 1e-4);
    if (core.a > 0.02) {
        rgb = core.rgb / core.a;
    }
    rgb = rgb * (1.0 - rim * 0.25);
    // A wet stroke catches the light: a soft sheen across its body, in
    // streaks along the brush, gone once it has dried.
    if (soaked > 0.0) {
        let gloss = smoothstep(0.55, 0.92, noise(rotate(sp, -NIB_ANGLE) * vec2<f32>(0.05, 0.4) + 8.0));
        rgb = mix(rgb, vec3<f32>(1.0), soaked * (0.06 + 0.3 * gloss) * clamp(body, 0.0, 1.0));
    }
    // Old ink: a little fainter, and browner on the light paper.
    let old = aged(px);
    let faded = alpha * (1.0 - 0.45 * old);
    let light = dot(frame.surface.rgb, vec3<f32>(0.3, 0.59, 0.11)) > 0.5;
    if (light) {
        rgb = mix(rgb, vec3<f32>(0.33, 0.22, 0.12), old * 0.6);
    } else {
        rgb = mix(rgb, frame.surface.rgb, old * 0.3);
    }
    return vec4<f32>(rgb * faded, faded);
}
