use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::alert_dispatch::{Alert, AlertLevel};
use crate::datadog_llm_obs::values::{as_text, mapping_field};
use crate::hanging_requests::check::AlertingHangingRequestCheck;

use super::deployment::deployment_latencies;
use super::helper::{SlowResponseParameters, SlowResponseRequest, slow_response_parameters};
use super::presentation::{float_text, rounded_to_two};
use super::sink::AlertSink;

#[derive(Debug, thiserror::Error)]
pub enum SlowResponseError {
    #[error(transparent)]
    ApiBase(#[from] ApiBaseError),
}

pub async fn slow_response_callback(
    check: &AlertingHangingRequestCheck,
    request: &SlowResponseRequest<'_>,
) -> Result<(), SlowResponseError> {
    let settings = check.settings.current();
    if !settings.alerting_configured {
        return Ok(());
    }
    let parameters = slow_response_parameters(check.api_base.as_ref(), request)?;
    if parameters.time_difference_seconds <= settings.alerting_threshold_seconds {
        return Ok(());
    }
    let (request_info, alerting_metadata) = slow_request_info(&parameters, request.litellm_params);
    let slow_message = format!(
        "`Responses are slow - {}s response time > Alerting threshold: {}s`{request_info}",
        rounded_to_two(parameters.time_difference_seconds),
        float_text(settings.alerting_threshold_seconds)
    );
    let empty_kwargs = IndexMap::new();
    let sink: &dyn AlertSink = check.sink.as_ref();
    sink.send_alert(Alert {
        message: slow_message.as_str(),
        level: AlertLevel::Low,
        alert_type: AlertType::LlmTooSlow,
        alerting_metadata: &alerting_metadata,
        user_info: None,
        request_model: Some(parameters.model.as_str()),
        api_base: parameters.api_base.as_deref(),
        kwargs: &empty_kwargs,
    })
    .await;
    Ok(())
}

fn slow_request_info(
    parameters: &SlowResponseParameters,
    litellm_params: Option<&IndexMap<String, Json>>,
) -> (String, IndexMap<String, Json>) {
    let mut request_info = format!(
        "\nRequest Model: `{}`\nAPI Base: `{}`\nMessages: `{}`",
        parameters.model,
        parameters.api_base.as_deref().unwrap_or(""),
        parameters.messages
    );
    let mut alerting_metadata = IndexMap::new();
    let metadata = litellm_params.and_then(|params| mapping_field(params, "metadata"));
    if let Some(metadata) = metadata {
        request_info = add_key_name_and_team(request_info, metadata);
        if let Some(latencies) = deployment_latencies(metadata) {
            request_info.push_str("\nAvailable Deployment Latencies\n");
            request_info.push_str(&latencies);
        }
        if let Some(Json::Object(entries)) = metadata.get("alerting_metadata") {
            alerting_metadata.clone_from(entries);
        }
    }
    (request_info, alerting_metadata)
}

fn add_key_name_and_team(request_info: String, metadata: &IndexMap<String, Json>) -> String {
    let mut info = request_info;
    if let Some(key_name) = metadata.get("user_api_key_alias").and_then(as_text) {
        let mut team = "";
        if let Some(alias) = metadata.get("user_api_key_team_alias").and_then(as_text) {
            team = alias;
        }
        info = format!("\n\nKey Name: `{key_name}`\nTeam: `{team}`{info}");
    }
    info
}
