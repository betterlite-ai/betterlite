use std::path::Path;
use std::time::Duration;

use crate::health_cycle::concurrency::DispatchMode;
use crate::health_cycle::rss::{RSS_STATUS_PATH, format_rss_mb, rss_mb_from, thread_count};

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
    let limit = max_concurrency.map_or_else(|| "None".to_owned(), |number| number.to_string());
    let rss = format_rss_mb(rss_mb_from(Path::new(RSS_STATUS_PATH)));
    tracing::debug!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        requested_model_count = counts.requested,
        post_model_filter_count = counts.post_filter,
        deduped_model_count = counts.deduped,
        max_concurrency = %limit,
        thread_count = thread_count(),
        rss_mb = %rss,
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
    let limit = max_concurrency.map_or_else(|| "None".to_owned(), |number| number.to_string());
    let rss = format_rss_mb(rss_mb_from(Path::new(RSS_STATUS_PATH)));
    tracing::debug!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        mode = mode.name(),
        model_count = model_count,
        max_concurrency = %limit,
        peak_in_flight = peak_in_flight,
        thread_count = thread_count(),
        rss_mb = %rss,
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
    let milliseconds = elapsed.as_secs_f64().mul_add(1000.0_f64, 0.0_f64);
    let duration = format!("{milliseconds:.2}");
    let rss = format_rss_mb(rss_mb_from(Path::new(RSS_STATUS_PATH)));
    tracing::debug!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        model_count = model_count,
        healthy_count = healthy_count,
        unhealthy_count = unhealthy_count,
        duration_ms = %duration,
        thread_count = thread_count(),
        rss_mb = %rss,
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
    let milliseconds = elapsed.as_secs_f64().mul_add(1000.0_f64, 0.0_f64);
    let duration = format!("{milliseconds:.2}");
    let rss = format_rss_mb(rss_mb_from(Path::new(RSS_STATUS_PATH)));
    tracing::error!(
        source = %context.source,
        cycle_id = %context.cycle_id,
        model_count = model_count,
        duration_ms = %duration,
        thread_count = thread_count(),
        rss_mb = %rss,
        error = %error,
        "health_check_cycle_failed",
    );
}
