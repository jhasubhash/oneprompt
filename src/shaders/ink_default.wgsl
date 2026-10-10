// The ink layer as it is, written in by hand.
fn theme_ink(px: vec2<f32>) -> vec4<f32> {
    return sample_at(px) * written(px, px);
}
