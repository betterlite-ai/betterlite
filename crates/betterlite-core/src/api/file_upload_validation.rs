use std::cmp::Ordering;
pub mod batch;
mod codec;
pub mod error;
pub mod failure;
mod json;
pub mod line_shape;
pub mod source;
mod tests;
pub mod upload;
const MEBIBYTE: u64 = 1024 * 1024;
fn exceeds(size_bytes: u128, limit_mb: i64) -> bool {
    match u64::try_from(limit_mb) {
        Ok(limit @ 1..) => match limit.checked_mul(MEBIBYTE) {
            Some(maximum) => size_bytes > u128::from(maximum),
            None => false,
        },
        Ok(0) | Err(_) => false,
    }
}
fn megabytes(size_bytes: u128) -> String {
    let scaled = size_bytes.saturating_mul(10);
    let divisor = u128::from(MEBIBYTE);
    let quotient = scaled.div_euclid(divisor);
    let remainder = scaled.rem_euclid(divisor);
    let rounded = match remainder.cmp(&divisor.div_euclid(2)) {
        Ordering::Less => quotient,
        Ordering::Greater => quotient.saturating_add(1),
        // Python's format rounds a tie to the even digit, as f64 formatting does there
        Ordering::Equal => match quotient.rem_euclid(2) {
            0 => quotient,
            _ => quotient.saturating_add(1),
        },
    };
    format!("{}.{}", rounded.div_euclid(10), rounded.rem_euclid(10))
}
