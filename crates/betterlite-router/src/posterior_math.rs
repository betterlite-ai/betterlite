#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

#[must_use]
pub fn mostly_non_ascii(non_ascii: usize, length: usize) -> bool {
    let count = f64::from(u32::try_from(non_ascii).unwrap_or(u32::MAX));
    let total = f64::from(u32::try_from(length.max(1)).unwrap_or(u32::MAX));
    count / total > 0.1_f64
}

// _posterior_mean at the global level: the +1/+2 smoothing over a statistic with no prior
#[must_use]
pub fn smoothed_mean(successes: f64, observations: f64) -> f64 {
    let smoothed_successes = successes + 1.0_f64;
    let smoothed_observations = observations + 2.0_f64;
    smoothed_successes / smoothed_observations
}

// _posterior_mean: an observation shifts the prior mean in proportion to its mass
#[must_use]
pub fn posterior_mean(successes: f64, observations: f64, prior_mass: f64, prior_mean: f64) -> f64 {
    let observed_successes = successes + prior_mass * prior_mean;
    let observed_mass = observations + prior_mass;
    observed_successes / observed_mass
}
