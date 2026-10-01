//! Stateless procedural randomness. Explicit wrapping arithmetic fixes the sequence.
const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_991.0;
fn integer(value: f64) -> Option<u64> {
    (value.is_finite() && (0.0..=MAX_EXACT_INTEGER).contains(&value) && value.fract() == 0.0)
        .then_some(value as u64)
}
fn sample(seed: u64, index: u64) -> f64 {
    let mut value = seed
        .wrapping_add(index.wrapping_mul(0x9e3779b97f4a7c15))
        .wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^= value >> 31;
    (value >> 11) as f64 / 9_007_199_254_740_992.0
}
pub(crate) fn random(seed: f64, index: f64) -> Option<f64> {
    Some(sample(integer(seed)?, integer(index)?))
}
pub(crate) fn noise(x: f64, seed: f64) -> Option<f64> {
    let seed = integer(seed)?;
    if !x.is_finite() || x.abs() > MAX_EXACT_INTEGER - 1.0 {
        return None;
    }
    let cell = x.floor();
    let fraction = x - cell;
    let weight = fraction * fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
    let left = sample(seed, (cell as i64) as u64);
    let right = sample(seed, ((cell as i64) + 1) as u64);
    Some(left * (1.0 - weight) + right * weight)
}
