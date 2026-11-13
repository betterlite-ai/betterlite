#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

const MIN_PROBABILITY: f64 = 1e-6;

pub(crate) fn threshold(base: f64, step: f64, extra_steps: u8) -> f64 {
    match extra_steps {
        0 => base,
        1 => base + step,
        extra => f64::from(extra).mul_add(step, base),
    }
}

pub(crate) fn max_threshold(base: f64, step: f64) -> Option<f64> {
    match threshold(base, step, 2) {
        adjusted if adjusted <= 1.0 => Some(adjusted),
        _ => None,
    }
}

pub(crate) fn select<'tier>(
    p_solve: f64,
    threshold: f64,
    efficient: &'tier str,
    capable: &'tier str,
) -> (&'tier str, bool) {
    if p_solve >= threshold || (threshold - p_solve).abs() <= f64::EPSILON {
        (efficient, true)
    } else {
        (capable, false)
    }
}

pub(crate) fn calibrate(slope: f64, intercept: f64, p_solve: f64) -> f64 {
    let clipped = p_solve.clamp(MIN_PROBABILITY, 1.0 - MIN_PROBABILITY);
    let log_difference = clipped.ln() - (-clipped).ln_1p();
    let scaled = slope * log_difference;
    let exponent = -(scaled + intercept);
    1.0 / (1.0 + exponent.exp())
}
