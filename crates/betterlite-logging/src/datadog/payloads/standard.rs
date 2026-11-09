use betterlite_models::decisions::Json;

use crate::datadog::apm::Tracer;
use crate::datadog::tagging::{Environment, RequestTagValue, TeamMetadata, tags};
// ast-grep-ignore: no-long-names
use crate::exporter_options::truncation::truncate_standard_logging_payload_content;
use crate::fields::payload::StandardLoggingPayload;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use crate::python_json;

use super::assembly::assemble_payload;
use super::error::DatadogPayloadError;
use super::status::DataDogStatus;
use super::types::DatadogPayload;

// The truncation create_datadog_logging_payload applies before its payload helper builds the envelope
pub fn standard_logging_to_datadog(
    standard_logging_object: &StandardLoggingPayload,
    status: DataDogStatus,
    environment: &Environment,
    tracer: &dyn Tracer,
) -> Result<DatadogPayload, DatadogPayloadError> {
    // ast-grep-ignore: no-long-names
    let truncated = truncate_standard_logging_payload_content(standard_logging_object);
    let message = safe_dumps(&truncated, DEFAULT_MAX_RECURSE_DEPTH, None)?;
    Ok(assemble_payload(
        environment,
        status,
        &payload_tags(environment, &truncated),
        message,
        tracer,
    ))
}

fn payload_tags(environment: &Environment, payload: &StandardLoggingPayload) -> Vec<String> {
    let request_tags: Vec<RequestTagValue> =
        payload.request_tags.iter().map(request_tag_value).collect();
    let team = TeamMetadata {
        user_api_key_team_alias: payload.metadata.user_api_key_team_alias.clone(),
        team_alias: payload.metadata.team_alias.clone(),
        user_api_key_team_id: payload.metadata.user_api_key_team_id.clone(),
        team_id: payload.metadata.team_id.clone(),
    };
    tags(environment, &request_tags, &team)
}

fn request_tag_value(value: &Json) -> RequestTagValue {
    match value {
        Json::Null => RequestTagValue::Null,
        Json::Bool(flag) => RequestTagValue::Bool(*flag),
        Json::Number(number) => RequestTagValue::Number(number.to_string()),
        Json::Text(text) => RequestTagValue::Text(text.clone()),
        // Python renders a list or object tag with str(); its quotes and spacing collapse anyway
        Json::List(_) | Json::Object(_) => RequestTagValue::Text(python_json::dumps(value)),
    }
}
