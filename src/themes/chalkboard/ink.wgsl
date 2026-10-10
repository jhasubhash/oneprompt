// Chalkboard's ink is chalk: it catches on the board's tooth, so strokes
// are broken and grainy (more on the low spots, where the chalk skips);
// their edges are soft with dust, and freshly written letters shed a little
// more dust that settles over a moment.

const DUST_TAPS: i32 = 8;

fn chalk_at(pos: vec2<f32>, sp: vec2<f32>) -> vec4<f32> {
    return sample_at(pos) * written(pos, sp);
}

fn theme_ink(px: vec2<f32>) -> vec4<f32> {
    let scale = max(frame.scale, 1.0);
    let lp = px / scale + board_offset();

    // The stick's edge is never straight: look the layer up through a small
    // jitter, and a stick of chalk is broader than a pen: keep the densest
    // of a few samples around.
    let jitter = (vec2<f32>(noise(lp * 0.8), noise(lp * 0.8 + 7.0)) - 0.5) * 0.9 * scale;
    var core = chalk_at(px + jitter, lp);
    let broad = 0.5 * scale;
    for (var k = 0; k < 4; k++) {
        let a = f32(k) * 1.5707963 + 0.4;
        let s = chalk_at(px + jitter + vec2<f32>(cos(a), sin(a)) * broad, lp);
        if (s.a > core.a) {
            core = s;
        }
    }

    // Dust: a faint halo of the stroke's colour, wider for fresh chalk.
    let age = cell_ink(px, vec2<f32>(0.0)).age;
    let fresh = 1.0 - clamp(age / max(look.settle, 1e-3), 0.0, 1.0);
    var dust = vec4<f32>(0.0);
    if (look.dust > 0.0) {
        let reach = (2.0 + 2.0 * fresh) * scale;
        let turn = hash(px) * 6.2831853;
        for (var k = 0; k < DUST_TAPS; k++) {
            let a = f32(k) * 6.2831853 / f32(DUST_TAPS) + turn;
            let r = reach * (0.4 + 0.6 * hash(px + f32(k) * 9.1));
            dust += chalk_at(px + vec2<f32>(cos(a), sin(a)) * r, lp);
        }
        dust /= f32(DUST_TAPS);
    }

    // Chalk sticks to the tooth's high spots and skips the low ones, the
    // more so at a stroke's thin edges.
    let high = tooth(lp);
    let thin = 1.0 - core.a;
    let catches = smoothstep(0.22 + 0.22 * look.grit * (1.0 + thin), 0.62, high);
    let body = core.a * mix(1.0, catches, look.grit * 0.85);
    // The haze is grainy too, and finer.
    let haze = dust.a * look.dust * (0.3 + 0.3 * fresh) * (0.4 + 0.8 * hash(floor(lp * 2.0)));

    let alpha = clamp(max(body, haze), 0.0, 1.0);
    var rgb = dust.rgb / max(dust.a, 1e-4);
    if (core.a > 0.02) {
        rgb = core.rgb / core.a;
    }
    return vec4<f32>(rgb * alpha, alpha);
}
