use super::events::{ERROR_METRIC_NAME, SUPPORTABILITY_METRIC_PREFIX};
use super::logger::NewRelicLogger;

pub(super) fn check_and_emit_periodic_metric(logger: &NewRelicLogger) {
    let now = (logger.clock)();
    if !logger.backoff.claim(now) {
        return;
    }
    emit_supportability_metric(logger);
}

fn emit_supportability_metric(logger: &NewRelicLogger) {
    let version = logger
        .version
        .as_deref()
        .filter(|version| !version.is_empty())
        .unwrap_or("unknown");
    let metric_name = format!("{SUPPORTABILITY_METRIC_PREFIX}{version}");
    let resolved = logger.agent.as_ref().and_then(|agent| agent.application());
    let Some(application) = resolved.filter(|candidate| candidate.enabled()) else {
        tracing::info!("New Relic application is not enabled; skipping metric recording.");
        return;
    };
    match application.record_custom_metric(&metric_name, 1) {
        Ok(()) => tracing::info!(
            metric = metric_name,
            "Emitted New Relic supportability metric"
        ),
        Err(error) => tracing::warn!(%error, "Failed to emit supportability metric"),
    }
}

pub(super) fn record_error_metric(logger: &NewRelicLogger) {
    if !logger.enabled {
        return;
    }
    check_and_emit_periodic_metric(logger);
    let resolved = logger.agent.as_ref().and_then(|agent| agent.application());
    let Some(application) = resolved.filter(|candidate| candidate.enabled()) else {
        return;
    };
    if let Err(error) = application.record_custom_metric(ERROR_METRIC_NAME, 1) {
        tracing::warn!(%error, "Failed to record New Relic error metric");
        logger.count_callback_failure();
    }
}
