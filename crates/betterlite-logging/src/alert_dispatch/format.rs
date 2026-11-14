use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::alert_settings::types::AlertType;

use super::Alert;
use crate::alert_batching::alert_delivery::PROXY_BASE_URL_ENV;
use crate::alert_digest::Clock;
use crate::clickhouse::clickhouse_logging::convert::coerced_text;

#[must_use]
pub fn format_message(
    alert: &Alert<'_>,
    clock: &dyn Clock,
    environment: &dyn EnvironmentLookup,
) -> String {
    let alert_type = alert.alert_type;
    let message = alert.message;
    let mut parts = if alert_type == AlertType::DailyReports
        || alert_type == AlertType::NewModelAdded
    {
        vec![format!("Alert type: `{alert_type}`{message}")]
    } else {
        let now = clock.now();
        let timestamp = now.format("%H:%M:%S");
        let level = alert.level.as_str();
        vec![format!(
            "Alert type: `{alert_type}`\nLevel: `{level}`\nTimestamp: `{timestamp}`\n\nMessage: {message}"
        )]
    };
    for (key, value) in alert.kwargs {
        parts.push(format!("\n\n{key}: `{}`\n\n", coerced_text(value)));
    }
    for (key, value) in alert.alerting_metadata {
        parts.push(format!(
            "\n\n*Alerting Metadata*: \n{key}: `{}`\n\n",
            coerced_text(value)
        ));
    }
    if let Some(proxy_base_url) = environment.lookup(PROXY_BASE_URL_ENV) {
        parts.push(format!("\n\nProxy URL: `{proxy_base_url}`"));
    }
    parts.concat()
}
