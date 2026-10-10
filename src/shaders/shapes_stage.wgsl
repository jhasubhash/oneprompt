// The shapes stage, in pixel space: cell backgrounds, cursor, underlines,
// box drawing, window controls, and whatever shapes the theme draws itself.
// Each instance is a box with a kind (theme::Shape):
//   0 filled rectangle
//   1 disc   (params.y = brush roughness, params.w = seed)
//   2 ring   (params.z = stroke width in px)
//   3 cross  (an "x", params.z = stroke width in px)
//   16 and up: the theme's own, drawn by its `theme_shape`

struct Instance {
    @location(0) rect: vec4<f32>,   // x, y, width, height in pixels
    @location(1) color: vec4<f32>,  // sRGB + alpha
    @location(2) params: vec4<f32>, // kind, then three values for the shape
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) params: vec4<f32>,
};

// What a theme's shape gets: its colour, the pixel's position in the box,
// the box's size, and the instance's params.
struct ShapeIn {
    color: vec4<f32>,
    local: vec2<f32>,
    size: vec2<f32>,
    params: vec4<f32>,
};

// Drawn as a 4-vertex triangle strip per instance.
@vertex
fn vs_main(@builtin(vertex_index) i: u32, inst: Instance) -> VsOut {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    let px = inst.rect.xy + corner * inst.rect.zw;
    let ndc = vec2<f32>(px.x / frame.resolution.x * 2.0 - 1.0, 1.0 - px.y / frame.resolution.y * 2.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = inst.color;
    out.local = corner * inst.rect.zw;
    out.size = inst.rect.zw;
    out.params = inst.params;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let kind = u32(in.params.x + 0.5);
    if (kind == 0u) {
        return in.color;
    }
    if (kind >= 16u) {
        return theme_shape(ShapeIn(in.color, in.local, in.size, in.params));
    }

    let p = in.local - in.size * 0.5;
    let radius = min(in.size.x, in.size.y) * 0.5 - 1.0;
    // Brush edges wobble with the direction, seamlessly around the circle.
    let dir = p / max(length(p), 1e-3);
    let seed = vec2<f32>(in.params.w * 7.31, in.params.w * 3.17);
    let wobble = (noise(dir * 1.7 + seed) - 0.5) + 0.5 * (noise(dir * 4.3 + seed * 1.9) - 0.5);
    let r = radius * (1.0 + in.params.y * wobble);
    let stroke = in.params.z;

    var d = 0.0;
    if (kind == 1u) {
        d = length(p) - r;
    } else if (kind == 2u) {
        d = abs(length(p) - (r - stroke * 0.5)) - stroke * 0.5;
    } else {
        let q = abs(p);
        d = max(abs(q.x - q.y) * 0.70710678 - stroke * 0.5, max(q.x, q.y) - radius);
    }
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
