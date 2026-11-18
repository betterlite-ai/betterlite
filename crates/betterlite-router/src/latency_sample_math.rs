#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};

// _average_latency: the mean of the window, and 0.0 before the first sample
#[must_use]
pub fn average(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0_f64;
    }
    let total: f64 = samples.iter().sum();
    let count = f64::from(u32::try_from(samples.len()).unwrap_or(u32::MAX));
    total / count
}

// _percentile_latency: the value at LiteLLM's one-based ceil(len * percentile) rank of the sorted window
#[must_use]
pub fn percentile(samples: &[f64], percentile: f64) -> Option<f64> {
    // an empty window has no rank; a non-finite percentile leaves the rank unrepresentable and converts to None
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let count = f64::from(u32::try_from(sorted.len()).unwrap_or(u32::MAX));
    let rank = i64::try_from(Decimal::from_f64((count * percentile).ceil())?).ok()?;
    let index = usize::try_from(rank.checked_sub(1)?).ok()?;
    sorted.get(index).copied()
}

// safe_divide_seconds: LiteLLM divides the response time by the completion tokens only when they are positive
#[must_use]
pub fn normalized(seconds: f64, completion_tokens: i64) -> Option<f64> {
    if completion_tokens <= 0 {
        return None;
    }
    let tokens = Decimal::from(completion_tokens).to_f64()?;
    Some(seconds / tokens)
}
