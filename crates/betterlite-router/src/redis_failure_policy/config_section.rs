use betterlite_config::sections::redis_failure_policy as section;

use super::Settings;

impl From<&section::Settings> for Settings {
    fn from(settings: &section::Settings) -> Self {
        Self {
            enabled: settings.enabled,
            failure_threshold: settings.failure_threshold,
            recovery_timeout: settings.recovery_timeout,
            timeout_min_duration: settings.timeout_min_duration,
            timeout_log_interval: settings.timeout_log_interval,
        }
    }
}
