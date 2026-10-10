// Logbook's ink is iron gall, put down with a steel nib: it feathers a
// little along the paper's fibers, pools at the ends of strokes, and changes
// colour as it dries, from the blue-black it goes on as (`wetness`) to the
// warm brown of old logs; old pages (`aged`) brown and fade further.

const FEATHER_TAPS: i32 = 6;

fn written_ink(pos: vec2<f32>, sp: vec2<f32>) -> vec4<f32> {
    return sample_at(pos) * written(pos, sp);
}

fn theme_ink(px: vec2<f32>) -> vec4<f32> {
    let scale = max(frame.scale, 1.0);
    let lp = px / scale + page_offset();
    let core = written_ink(px, lp);

    // Feathering: faint tendrils of ink along the fibers.
    var feather = vec4<f32>(0.0);
    if (look.feather > 0.0) {
        let dir = fiber_dir(lp);
        let turn = hash(px) * 6.2831853;
        for (var k = 0; k < FEATHER_TAPS; k++) {
            let along = (f32(k) / f32(FEATHER_TAPS - 1) - 0.5) * 2.0;
            let side = vec2<f32>(-dir.y, dir.x) * (hash(px + f32(k)) - 0.5) * 0.8;
            let offset = (dir * along * 2.2 + side + vec2<f32>(cos(turn), sin(turn)) * 0.3) * scale * look.feather;
            feather += written_ink(px + offset, lp);
        }
        feather /= f32(FEATHER_TAPS);
    }
    let tendrils = smoothstep(0.4, 0.8, noise(lp * vec2<f32>(0.3, 1.2)));
    let alpha = clamp(max(core.a * 1.15, feather.a * look.feather * (0.25 + 0.4 * tendrils)), 0.0, 1.0);
    var rgb = feather.rgb / max(feather.a, 1e-4);
    if (core.a > 0.02) {
        rgb = core.rgb / core.a;
    }

    // Iron gall: blue-black while wet, browning as it dries; old ink
    // browner and fainter still. Coloured inks keep their own hue.
    let wet = wetness(px);
    let old = aged(px);
    let night = look.night > 0.5;
    let plain = distance(rgb, frame.ink.rgb) < 0.12;
    if (plain && !night) {
        rgb = mix(rgb, vec3<f32>(0.1, 0.12, 0.22), wet * 0.7);
        rgb = mix(rgb, vec3<f32>(0.36, 0.24, 0.14), old * 0.7);
    }
    if (night) {
        // By lamplight fresh ink glints a little brighter.
        rgb = mix(rgb, min(rgb * 1.2, vec3<f32>(1.0)), wet * 0.5);
        rgb = mix(rgb, frame.surface.rgb, old * 0.3);
    }
    let faded = alpha * (1.0 - 0.4 * old);
    return vec4<f32>(rgb * faded, faded);
}
