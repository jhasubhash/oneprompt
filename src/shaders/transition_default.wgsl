// A plain crossfade.
fn theme_transition(px: vec2<f32>, now: vec4<f32>, before: vec4<f32>) -> vec4<f32> {
    return mix(before, now, frame.progress);
}
