#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

const WEIGHTS_SUM_TOLERANCE: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightTotal {
    pub total: f64,
    pub within_tolerance: bool,
}

#[must_use]
pub fn weights_total(quality: f64, cost: f64) -> WeightTotal {
    let total = quality + cost;
    let deviation = (total - 1.0_f64).abs();
    WeightTotal {
        total,
        within_tolerance: deviation <= WEIGHTS_SUM_TOLERANCE,
    }
}
