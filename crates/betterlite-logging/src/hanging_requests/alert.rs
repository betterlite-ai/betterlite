use betterlite_config::sections::alert_settings::types::AlertType;
use indexmap::IndexMap;

use crate::alert_dispatch::{Alert, AlertLevel};
use crate::alert_settings::records::HangingRequestData;
use crate::latency_alerts::presentation::float_text;
use crate::latency_alerts::settings::RequestAlertSettings;
use crate::latency_alerts::sink::AlertSink;

pub async fn send_hanging_request_alert(
    sink: &dyn AlertSink,
    settings: &RequestAlertSettings,
    data: &HangingRequestData,
) {
    let request_info = format!(
        "Request Model: `{}`\nAPI Base: `{}`\nKey Alias: `{}`\nTeam Alias: `{}`",
        data.model,
        data.api_base.as_deref().unwrap_or(""),
        data.key_alias.as_deref().unwrap_or(""),
        data.team_alias.as_deref().unwrap_or("")
    );
    let message = format!(
        "`Requests are hanging - {}s+ request time`\n{request_info}",
        float_text(settings.alerting_threshold_seconds)
    );
    let empty = IndexMap::new();
    let alerting_metadata = data.alerting_metadata.as_ref().unwrap_or(&empty);
    sink.send_alert(Alert {
        message: message.as_str(),
        level: AlertLevel::Medium,
        alert_type: AlertType::LlmRequestsHanging,
        alerting_metadata,
        user_info: None,
        request_model: Some(data.model.as_str()),
        api_base: data.api_base.as_deref(),
        kwargs: &empty,
    })
    .await;
}
