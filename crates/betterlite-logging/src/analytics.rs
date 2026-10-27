#[expect(
    clippy::disallowed_types,
    reason = "analytics projections preserve provider JSON wire fields"
)]
pub mod lunary;
#[expect(
    clippy::disallowed_types,
    reason = "analytics projections preserve provider JSON wire fields"
)]
pub mod wandb;

#[cfg(test)]
#[path = "analytics/tests.rs"]
mod sdk_analytics;
