use num_traits::ToPrimitive;

pub fn embedding_f64_to_f32(value: f64) -> f32 {
    value.to_f32().unwrap_or_else(|| {
        if value.is_sign_negative() {
            f32::MIN
        } else {
            f32::MAX
        }
    })
}

pub fn embedding_vec_f64_to_f32(values: &[f64]) -> Vec<f32> {
    values.iter().copied().map(embedding_f64_to_f32).collect()
}
