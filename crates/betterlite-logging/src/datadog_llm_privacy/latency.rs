use std::ops::{Add as _, Mul as _, Sub as _};

use crate::datadog_llm_obs::span_records::DDLLMObsLatencyMetrics;
use crate::fields::payload::StandardLoggingPayload;

const MILLIS_PER_SECOND: f64 = 1000.0;

// CompletionStartTime - StartTime is the time to first token; a call without one falls back to EndTime
#[must_use]
pub fn time_to_first_token_seconds(
    start_time: Option<f64>,
    completion_start_time: Option<f64>,
    end_time: Option<f64>,
) -> f64 {
    match (start_time, completion_start_time, end_time) {
        (Some(start), Some(completion_start), _) => completion_start.sub(start),
        (Some(start), None, Some(end)) => end.sub(start),
        (None, _, _) | (Some(_), None, None) => 0.0_f64,
    }
}

#[must_use]
pub fn latency_metrics(payload: &StandardLoggingPayload) -> DDLLMObsLatencyMetrics {
    let mut metrics = DDLLMObsLatencyMetrics::default();
    let start = payload.start_time.to_string().parse::<f64>().ok();
    let completion_start = payload
        .completion_start_time
        .to_string()
        .parse::<f64>()
        .ok();
    let end = payload.end_time.to_string().parse::<f64>().ok();
    let seconds = time_to_first_token_seconds(start, completion_start, end);
    if seconds > 0.0_f64 {
        metrics.time_to_first_token_ms = Some(seconds.mul(MILLIS_PER_SECOND));
    }
    if let Some(overhead) = payload.hidden_params.litellm_overhead_time_ms {
        metrics.litellm_overhead_time_ms = overhead.0.to_string().parse::<f64>().ok();
    }
    let mut total = 0.0_f64;
    for record in payload.guardrail_information.iter().flatten() {
        if let Some(Some(duration)) = record.duration
            && let Ok(duration_seconds) = duration.0.to_string().parse::<f64>()
        {
            total = total.add(duration_seconds);
        }
    }
    if total > 0.0_f64 {
        metrics.guardrail_overhead_time_ms = Some(total.mul(MILLIS_PER_SECOND));
    }
    metrics
}

mod tests;
