pub mod cache_size;
pub mod changes;
pub mod error;
pub mod ports;
pub mod register;
pub mod schedule;
pub mod trigger;

mod tests;

pub const SPEND_LOG_CLEANUP_JOB_ID: &str = "spend_log_cleanup_job";

pub const CLEANUP_SCHEDULE_KEYS: [&str; 6] = [
    RETENTION_PERIOD_KEY,
    AUTOROUTER_RETENTION_KEY,
    HEALTH_RETENTION_KEY,
    TAG_SPEND_RETENTION_KEY,
    CLEANUP_CRON_KEY,
    RETENTION_INTERVAL_KEY,
];

pub(crate) const RETENTION_PERIOD_KEY: &str = "maximum_spend_logs_retention_period";
pub(crate) const AUTOROUTER_RETENTION_KEY: &str = "maximum_autorouter_session_retention_period";
pub(crate) const HEALTH_RETENTION_KEY: &str = "maximum_health_check_retention_period";
pub(crate) const TAG_SPEND_RETENTION_KEY: &str = "maximum_daily_tag_spend_retention_period";
pub(crate) const CLEANUP_CRON_KEY: &str = "maximum_spend_logs_cleanup_cron";
pub(crate) const RETENTION_INTERVAL_KEY: &str = "maximum_spend_logs_retention_interval";
