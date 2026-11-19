#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

// _get_available_deployments: how many of the ascending scores are at most lowest + lowest_latency_buffer * lowest
#[must_use]
pub fn valid_count(scores: &[f64], lowest_latency_buffer: f64) -> usize {
    let Some(lowest) = scores.first().copied() else {
        return 0;
    };
    let buffer = lowest_latency_buffer * lowest;
    let highest = lowest + buffer;
    scores.iter().take_while(|score| **score <= highest).count()
}
