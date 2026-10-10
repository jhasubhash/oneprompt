// The transition stage: copies the finished scene to the window, or while a
// reveal or switch runs, lets the theme's `theme_transition` show it over
// what was there before (a solid colour on opening, the last frame shown
// when switching).

@group(0) @binding(3) var scene: texture_2d<f32>;
@group(0) @binding(4) var previous: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> FullScreen {
    return full_screen(i);
}

@fragment
fn fs_main(in: FullScreen) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.pos.xy);
    let now = textureLoad(scene, texel, 0);
    if (frame.progress >= 1.0) {
        return now;
    }
    var before = textureLoad(previous, texel, 0);
    if (frame.from_color.w > 0.5) {
        before = vec4<f32>(frame.from_color.rgb, 1.0);
    }
    return theme_transition(in.pos.xy, now, before);
}
