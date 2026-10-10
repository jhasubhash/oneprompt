// The surface stage: the sheet the text sits on, drawn by the theme's
// `surface` (in logical pixels, so it looks the same on any display).

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> FullScreen {
    return full_screen(i);
}

@fragment
fn fs_main(in: FullScreen) -> @location(0) vec4<f32> {
    let px = in.pos.xy / max(frame.scale, 1.0);
    let uv = in.pos.xy / frame.resolution;
    return vec4<f32>(surface(px, uv), 1.0);
}
