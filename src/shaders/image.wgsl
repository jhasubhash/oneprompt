// Draws a picture (a theme's preview in the picker, or a picture a program
// showed in the terminal) into a box, scaled down smoothly (a small box
// filter over the source), with rounded corners. Each instance is a
// theme::Shape: rect, colour (its alpha fades the picture), params.y =
// corner radius in pixels.

@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var picture_sampler: sampler;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) alpha: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32, inst: Instance) -> VsOut {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    let px = inst.rect.xy + corner * inst.rect.zw;
    var out: VsOut;
    out.pos = vec4<f32>(px.x / frame.resolution.x * 2.0 - 1.0, 1.0 - px.y / frame.resolution.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    out.local = corner * inst.rect.zw;
    out.size = inst.rect.zw;
    out.radius = inst.params.y;
    out.alpha = inst.color.a;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let source = vec2<f32>(textureDimensions(picture));
    // How many source pixels one of ours covers; sample a grid across it.
    let step = source / max(in.size, vec2<f32>(1.0)) / source;
    var sum = vec4<f32>(0.0);
    for (var j = 0; j < 3; j++) {
        for (var i = 0; i < 3; i++) {
            let offset = (vec2<f32>(f32(i), f32(j)) - 1.0) * step / 3.0;
            sum += textureSampleLevel(picture, picture_sampler, in.uv + offset, 0.0);
        }
    }
    let q = abs(in.local - in.size * 0.5) - in.size * 0.5 + in.radius;
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
    let picture = sum / 9.0;
    return vec4<f32>(picture.rgb, picture.a * clamp(0.5 - d, 0.0, 1.0) * in.alpha);
}
