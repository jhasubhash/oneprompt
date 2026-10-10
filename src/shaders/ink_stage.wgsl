// The ink stage: composites the ink layer (text, line art, window controls;
// premultiplied alpha on a transparent target) onto the surface through the
// theme's `theme_ink`, which decides how ink behaves on it. This part gives
// it the layer and the hand-writing state: new characters are written in
// along their pen path (trace.rs); glyphs without one get a ragged sweep.

@group(0) @binding(2) var ink: texture_2d<f32>;
@group(0) @binding(3) var ink_sampler: sampler;
// Per grid cell: when it was last written (seconds) and its pen-path slot.
@group(0) @binding(4) var births: texture_2d<f32>;
// Pen order per glyph pixel, 1/255..1 along the path; 0 = no path.
@group(0) @binding(5) var pen_order: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> FullScreen {
    return full_screen(i);
}

@fragment
fn fs_main(in: FullScreen) -> @location(0) vec4<f32> {
    return theme_ink(in.pos.xy);
}

fn sample_at(px: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(ink, ink_sampler, px / frame.resolution, 0.0);
}

struct CellInk {
    age: f32,     // seconds since the cell was written (may be negative)
    order: f32,   // where this pixel lies on the glyph's pen path, 0..1
    traced: bool, // false: no path, use the sweep instead
};

// Writing state of the ink at `px`, as part of the cell at `px + nudge`.
// Pixels outside the grid (window controls, notices) are old, i.e. dry.
fn cell_ink(px: vec2<f32>, nudge: vec2<f32>) -> CellInk {
    let g = (px + nudge - frame.grid_origin) / frame.cell;
    if (frame.write_time <= 0.0 || any(g < vec2<f32>(0.0)) || any(g >= frame.grid_size)) {
        return CellInk(1.0e6, 0.0, false);
    }
    let cell = textureLoad(births, vec2<i32>(g), 0);
    let age = frame.time - cell.r;
    if (age >= frame.write_time) {
        return CellInk(age, 0.0, false);
    }
    let slot = u32(cell.g + 0.5);
    if (slot > 0u) {
        let columns = u32(frame.atlas_columns);
        let corner = vec2<f32>(f32(slot % columns), f32(slot / columns)) * frame.slot_size;
        // +1: the glyph sits one pixel into its slot (margin for overhang).
        let local = clamp(px - (frame.grid_origin + floor(g) * frame.cell) + vec2<f32>(1.0, 0.0),
                          vec2<f32>(0.0), frame.slot_size - 1.0);
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
// reaches it, 1 once written (`sp`: `pos` with the sheet's seed offset, for
// the sweep's raggedness). Apply it to every sample, so effects spreading
// ink can't pick up a character (or part of one) before it is due.
//
// Glyphs overhang their cells (serifs, left bearings), so ink near a cell's
// edge may belong to the neighbouring character. Such ink shows only once
// its own cell and the neighbour on that side have both written it.
fn written(pos: vec2<f32>, sp: vec2<f32>) -> f32 {
    let own = written_as(pos, sp, vec2<f32>(0.0));
    let next = written_as(pos, sp, vec2<f32>(frame.cell.x * 0.3, 0.0));
    let previous = written_as(pos, sp, vec2<f32>(-frame.cell.x * 0.3, 0.0));
    return min(own, min(next, previous));
}

fn written_as(pos: vec2<f32>, sp: vec2<f32>, nudge: vec2<f32>) -> f32 {
    let c = cell_ink(pos, nudge);
    if (c.age >= frame.write_time) {
        return 1.0;
    }
    let progress = clamp(c.age / max(frame.write_time, 1e-3), 0.0, 1.0);
    if (c.traced) {
        // The pen tip: a short soft front moving along the path.
        return clamp((progress * 1.12 - c.order) * 18.0, 0.0, 1.0);
    }
    // Kept above zero so nothing shows before the pen arrives.
    let ragged = max(c.order + (noise(sp * 0.4) - 0.5) * 0.35
        + (noise(rotate(sp, 0.35) * vec2<f32>(0.08, 0.9)) - 0.5) * 0.25, 0.02);
    return clamp((progress * 1.45 - ragged) * 9.0, 0.0, 1.0);
}
