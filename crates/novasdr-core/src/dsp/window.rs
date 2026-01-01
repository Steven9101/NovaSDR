pub fn hann_window(size: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; size];
    let denom = size as f32;
    for (i, v) in out.iter_mut().enumerate() {
        *v = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / denom).cos());
    }
    out
}
