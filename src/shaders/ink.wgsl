// Composites the ink layer (text, line art, window controls; premultiplied
// alpha on a transparent target) onto the paper, the way ink behaves on it:
//   bleed  - ink wicks past strokes in patches, further along the fibers;
//            stroke edges are ragged, so no two letters look the same
//   nib    - glyphs are swept with a slanted brush, so strokes across it come
//            out thick and strokes along it thin, with pressure varying
//   grain  - dry-brush streaks: the core of a stroke is not perfectly solid
//   edge   - pigment pools where a stroke dries, darkening its rim
//   write  - new characters are written by hand: each glyph is uncovered
//            along its pen path (trace.rs), then its bleed spreads as the ink
//            dries. Glyphs without a path get a ragged brush sweep.

struct Params {
    resolution: vec2<f32>,
    bleed: f32,       // reach of the bleed in physical pixels
    grain: f32,       // 0..1
    edge: f32,        // 0..1
    seed: f32,        // per-session, matches the paper
    nib: f32,         // length of the brush nib in physical pixels
    time: f32,        // seconds since start, same clock as the births
    grid_origin: vec2<f32>, // top-left of the terminal grid, pixels
    cell: vec2<f32>,        // cell size, pixels
    grid_size: vec2<f32>,   // columns, rows
    write_time: f32,        // seconds to write one character; 0 = off
    atlas_columns: f32,     // slots per row in the pen-order atlas
    slot_size: vec2<f32>,   // atlas slot size, pixels (cell + 2 px margin)
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Params;
@group(0) @binding(1) var ink: texture_2d<f32>;
@group(0) @binding(2) var ink_sampler: sampler;
// Per grid cell: when it was last written (seconds) and its pen-path slot.
@group(0) @binding(3) var births: texture_2d<f32>;
// Pen order per glyph pixel, 1/255..1 along the path; 0 = no path.
@group(0) @binding(4) var pen_order: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), s.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), s.x),
        s.y,
    );
}

fn sample_at(px: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(ink, ink_sampler, px / u.resolution, 0.0);
}

const TAPS: i32 = 12;
const NIB_TAPS: i32 = 3;
// The brush is held tilted up to the right, as for most calligraphy.
const NIB_ANGLE: f32 = -0.62;
const NIB_DIR: vec2<f32> = vec2<f32>(0.8139, -0.5810); // (cos, sin) of NIB_ANGLE

fn rotate(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

struct CellInk {
    age: f32,     // seconds since the cell was written (may be negative)
    order: f32,   // where this pixel lies on the glyph's pen path, 0..1
    traced: bool, // false: no path, use the sweep instead
};

// Writing state of the ink at `px`, as part of the cell at `px + nudge`.
// Pixels outside the grid (window controls, notices) are old, i.e. dry.
fn cell_ink(px: vec2<f32>, nudge: vec2<f32>) -> CellInk {
    let g = (px + nudge - u.grid_origin) / u.cell;
    if (u.write_time <= 0.0 || any(g < vec2<f32>(0.0)) || any(g >= u.grid_size)) {
        return CellInk(1.0e6, 0.0, false);
    }
    let cell = textureLoad(births, vec2<i32>(g), 0);
    let age = u.time - cell.r;
    if (age >= u.write_time) {
        return CellInk(age, 0.0, false);
    }
    let slot = u32(cell.g + 0.5);
    if (slot > 0u) {
        let columns = u32(u.atlas_columns);
        let corner = vec2<f32>(f32(slot % columns), f32(slot / columns)) * u.slot_size;
        // +1: the glyph sits one pixel into its slot (margin for overhang).
        let local = clamp(px - (u.grid_origin + floor(g) * u.cell) + vec2<f32>(1.0, 0.0),
                          vec2<f32>(0.0), u.slot_size - 1.0);
        let order = textureLoad(pen_order, vec2<i32>(corner + local), 0).r;
        if (order > 0.0) {
            return CellInk(age, order, true);
        }
    }
    // Sweep: position along the brush through the cell, left to right and
    // slightly rising.
    let along = dot(fract(g) - 0.5, vec2<f32>(0.94, -0.34)) + 0.5;
    return CellInk(age, along, false);
}

// How much of the ink at `pos` has been written yet: 0 before the pen
// reaches it, 1 once written. Applied to every sample, so the nib and bleed
// can't pick up ink from a character (or part of one) before it is due.
//
// Glyphs overhang their cells (serifs, left bearings), so ink near a cell's
// edge may belong to the neighbouring character. Such ink shows only once
// its own cell and the neighbour on that side have both written it.
fn written(pos: vec2<f32>, sp: vec2<f32>) -> f32 {
    let own = written_as(pos, sp, vec2<f32>(0.0));
    let next = written_as(pos, sp, vec2<f32>(u.cell.x * 0.3, 0.0));
    let previous = written_as(pos, sp, vec2<f32>(-u.cell.x * 0.3, 0.0));
    return min(own, min(next, previous));
}

fn written_as(pos: vec2<f32>, sp: vec2<f32>, nudge: vec2<f32>) -> f32 {
    let c = cell_ink(pos, nudge);
    if (c.age >= u.write_time) {
        return 1.0;
    }
    let progress = clamp(c.age / max(u.write_time, 1e-3), 0.0, 1.0);
    if (c.traced) {
        // The pen tip: a short soft front moving along the path.
        return clamp((progress * 1.12 - c.order) * 18.0, 0.0, 1.0);
    }
    // Kept above zero so nothing shows before the pen arrives.
    let ragged = max(c.order + (noise(sp * 0.4) - 0.5) * 0.35
        + (noise(rotate(sp, 0.35) * vec2<f32>(0.08, 0.9)) - 0.5) * 0.25, 0.02);
    return clamp((progress * 1.45 - ragged) * 9.0, 0.0, 1.0);
}

fn ink_at(pos: vec2<f32>, sp: vec2<f32>) -> vec4<f32> {
    return sample_at(pos) * written(pos, sp);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px = in.pos.xy;
    if (u.bleed <= 0.0 && u.grain <= 0.0 && u.nib <= 0.0) {
        return sample_at(px);
    }
    let sp = px + vec2<f32>(fract(u.seed * 0.137) * 400.0, fract(u.seed * 0.291) * 400.0);

    // Ragged edges: look the ink up through a small wobble, so stroke
    // outlines are uneven instead of the font's perfect curves.
    let wobble = (vec2<f32>(noise(sp * 0.45), noise(sp * 0.45 + 17.0)) - 0.5) * u.bleed * 0.9;

    // Brush nib: sweep the glyph along a slanted segment and keep the
    // densest ink. Pressure (the sweep length) swells and fades slowly
    // across the page, so stroke weight is never quite uniform.
    let pressure = 0.6 + 0.6 * noise(sp * 0.025 + 11.0);
    let sweep = NIB_DIR * u.nib * pressure * 0.5;
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
    if (u.bleed > 0.0) {
        let jitter = hash(px) * 6.2831853;
        let reach = u.bleed * (0.25 + 1.75 * amount);
        for (var k = 0; k < TAPS; k++) {
            let angle = f32(k) * 6.2831853 / f32(TAPS) + jitter;
            let r = reach * (0.3 + 0.7 * hash(px + f32(k) * 13.7));
            let offset = rotate(vec2<f32>(cos(angle) * 1.7, sin(angle) * 0.55) * r, fiber_angle);
            halo += ink_at(px + offset, sp);
        }
        halo /= f32(TAPS);
    }

    // Capillary feathering: the halo breaks up into fine tendrils instead
    // of a smooth glow.
    let capillary = smoothstep(0.35, 0.75, noise(rotate(sp, fiber_angle) * vec2<f32>(0.25, 1.1)));
    let wick = halo.a * mix(0.15, 1.0, capillary) * (0.35 + 0.65 * amount);
    // (The halo's spread is applied below, once `spread` is known.)

    // Ink soaks in: antialiased edges of thin strokes saturate towards solid;
    // the load of ink varies a little, and dry-brush grain thins it in places.
    // Dry-brush streaks run with the brush: fine across it, long along it.
    let streak = noise(rotate(sp, -NIB_ANGLE) * vec2<f32>(0.15, 1.3) + 3.1);
    let grain = 1.0 - u.grain * smoothstep(0.4, 0.95, streak * 0.75 + hash(px) * 0.25);
    let load = 0.9 + 0.1 * noise(sp * 0.02 + 5.0);
    let body = clamp(core.a * 1.3, 0.0, 1.0) * grain * load;

    // After the brush front (applied per sample above), the bleed spreads
    // and the wet ink settles over a few write times.
    let age = cell_ink(px, vec2<f32>(0.0)).age;
    var spread = 1.0;
    var wet = 0.0;
    if (age < u.write_time * 4.0) {
        spread = smoothstep(0.0, 1.0, age / (u.write_time * 3.0));
        wet = 1.0 - clamp(age / (u.write_time * 4.0), 0.0, 1.0);
    }

    // Pigment pools at the rim: strongest where the halo is half covered.
    let rim = clamp(halo.a * (1.0 - halo.a) * 4.0, 0.0, 1.0) * u.edge;

    // Wet ink is a touch heavier and its bleed is still creeping outwards.
    let alpha = clamp(
        max(body * (1.0 + 0.25 * wet), wick * 0.85 * spread) + rim * 0.2 * spread * (1.0 - body),
        0.0,
        1.0,
    );
    // Colour: the stroke's own colour, or the spread colour in the halo.
    var rgb = halo.rgb / max(halo.a, 1e-4);
    if (core.a > 0.02) {
        rgb = core.rgb / core.a;
    }
    rgb = rgb * (1.0 - rim * 0.25);
    return vec4<f32>(rgb * alpha, alpha);
}
