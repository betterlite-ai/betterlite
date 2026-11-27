use betterlite_config::sections::prometheus_budget_refresh::Settings;

mod tests;

pub const JOB_ID: &str = "prometheus_budget_metrics_job";

// The scheduler runs it every interval_minutes, first one interval after startup, and replaces any job with the same id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetRefreshJob<Logger> {
    pub id: &'static str,
    pub interval_minutes: i64,
    pub logger: Logger,
}

// LiteLLM's initialize_budget_metrics_cron_job: the setting is checked before the callback list is read
#[must_use]
pub fn budget_refresh_job<Logger: Clone>(
    settings: &Settings,
    loggers: &[Logger],
) -> Option<BudgetRefreshJob<Logger>> {
    if !settings.initialize_budget_metrics {
        return None;
    }
    tracing::debug!("found {} prometheus loggers", loggers.len());
    let logger = loggers.first()?.clone();
    tracing::debug!(
        "Initializing remaining budget metrics as a cron job executing every {} minutes",
        settings.refresh_interval_minutes
    );
    Some(BudgetRefreshJob {
        id: JOB_ID,
        interval_minutes: settings.refresh_interval_minutes,
        logger,
    })
}
