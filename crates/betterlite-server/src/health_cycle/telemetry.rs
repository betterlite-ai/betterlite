use std::time::Duration;

use crate::health_cycle::concurrency::DispatchMode;
use crate::health_cycle::rss::{rss_mb_for_log, thread_count};

const DEFAULT_SOURCE: &str = "unknown";
const DEFAULT_CYCLE_ID: &str = "unknown";

// instrumentation_context: the background loop's cycle identity and whether it asked for these lines at all
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstrumentationContext {
    pub enabled: bool,
    pub cycle_id: String,
    pub source: String,
}

impl Default for InstrumentationContext {
    fn default() -> Self {
        Self {
            enabled: false,
            cycle_id: DEFAULT_CYCLE_ID.to_owned(),
            source: DEFAULT_SOURCE.to_owned(),
        }
    }
}

// The three model counts perform_health_check reads before it dispatches: requested, after the model filter, after the id dedupe
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CycleCounts {
    pub requested: usize,
    pub post_filter: usize,
    pub deduped: usize,
}

// time.monotonic() milliseconds with %.2f, which mul_add keeps clear of float arithmetic lints
#[must_use]
pub fn elapsed_ms(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64().mul_add(1000.0_f64, 0.0_f64)
}

// percent-s of None is "None", and of an integer its digits, as the LiteLLM line logs it
#[must_use]
pub fn option_label(value: Option<i64>) -> String {
    value.map_or_else(|| "None".to_owned(), |number| number.to_string())
}

pub fn cycle_skipped(context: &InstrumentationContext, reason: &str) {
    if !context.enabled {
        return;
    }
    tracing::debug!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        reason = %reason,
        "health_check_cycle_skipped",
    );
}

pub fn cycle_start(
    context: &InstrumentationContext,
    counts: &CycleCounts,
    max_concurrency: Option<i64>,
) {
    if !context.enabled {
        return;
    }
    tracing::debug!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        requested_model_count = counts.requested,
        post_model_filter_count = counts.post_filter,
        deduped_model_count = counts.deduped,
        max_concurrency = %option_label(max_concurrency),
        thread_count = thread_count(),
        rss_mb = %rss_mb_for_log(),
        "health_check_cycle_start",
    );
}

pub fn dispatch_summary(
    context: &InstrumentationContext,
    mode: DispatchMode,
    model_count: usize,
    max_concurrency: Option<i64>,
    peak_in_flight: usize,
) {
    if !context.enabled {
        return;
    }
    tracing::debug!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        mode = mode.name(),
        model_count = model_count,
        max_concurrency = %option_label(max_concurrency),
        peak_in_flight = peak_in_flight,
        thread_count = thread_count(),
        rss_mb = %rss_mb_for_log(),
        "health_check_dispatch_summary",
    );
}

pub fn cycle_complete(
    context: &InstrumentationContext,
    model_count: usize,
    healthy_count: usize,
    unhealthy_count: usize,
    elapsed: Duration,
) {
    if !context.enabled {
        return;
    }
    tracing::debug!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        model_count = model_count,
        healthy_count = healthy_count,
        unhealthy_count = unhealthy_count,
        duration_ms = %format!("{:.2}", elapsed_ms(elapsed)),
        thread_count = thread_count(),
        rss_mb = %rss_mb_for_log(),
        "health_check_cycle_complete",
    );
}

pub fn cycle_failed(
    context: &InstrumentationContext,
    model_count: usize,
    elapsed: Duration,
    error: &str,
) {
    if !context.enabled {
        return;
    }
    tracing::error!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        model_count = model_count,
        duration_ms = %format!("{:.2}", elapsed_ms(elapsed)),
        thread_count = thread_count(),
        rss_mb = %rss_mb_for_log(),
        error = %error,
        "health_check_cycle_failed",
    );
}
