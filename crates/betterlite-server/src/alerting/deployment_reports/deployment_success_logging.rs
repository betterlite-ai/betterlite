use std::ops::{Div as _, Sub as _};

use betterlite_config::sections::alert_settings::types::AlertType;

use super::deployment_metrics::{DailyReportCache, DeploymentMetrics, update_daily_reports};
use crate::model_deprecation::clock::UtcClock;

// async_log_success_event's inputs: the deployment, the call's seconds, and the completion tokens a ModelResponse carried.
pub struct SuccessEvent<'event> {
    pub deployment_id: &'event str,
    pub start_time: f64,
    pub end_time: f64,
    pub completion_tokens: Option<i64>,
}

// async_log_success_event: with daily_reports enabled, record the latency per output token for this deployment.
pub async fn log_success_event<Cache, Clock>(
    cache: &Cache,
    clock: &Clock,
    alert_types: &[AlertType],
    event: &SuccessEvent<'_>,
) -> u8
where
    Cache: DailyReportCache,
    Clock: UtcClock + Send + Sync,
{
    let Some(metrics) = success_event_metrics(clock, alert_types, event) else {
        return 0;
    };
    update_daily_reports(cache, metrics).await
}

// The record the success event would store, or none when daily_reports is not an alert type.
#[must_use]
pub fn success_event_metrics<'event, Clock>(
    clock: &Clock,
    alert_types: &[AlertType],
    event: &SuccessEvent<'event>,
) -> Option<DeploymentMetrics<'event>>
where
    Clock: UtcClock + Send + Sync,
{
    if !alert_types.contains(&AlertType::DailyReports) {
        return None;
    }
    Some(DeploymentMetrics {
        id: event.deployment_id,
        failed_request: false,
        latency_per_output_token: Some(per_output_token(event)),
        updated_at: clock.now(),
    })
}

// completion_tokens > 0 divides the elapsed seconds; every other count keeps the whole response time.
fn per_output_token(event: &SuccessEvent<'_>) -> f64 {
    let elapsed = event.end_time.sub(event.start_time);
    let Some(tokens) = event.completion_tokens.filter(|tokens| *tokens > 0) else {
        return elapsed;
    };
    num_traits::cast::<i64, f64>(tokens).map_or(elapsed, |count| elapsed.div(count))
}

mod tests;
