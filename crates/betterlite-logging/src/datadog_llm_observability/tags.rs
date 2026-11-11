use betterlite_models::decisions::Json;

use crate::datadog::tagging::{Environment, RequestTagValue, TeamMetadata, tags};
use crate::fields::payload::StandardLoggingPayload;
use crate::python_json;

// get_datadog_tags(standard_logging_object): the base tags, the request tags and the team tag
#[must_use]
pub fn span_tags(environment: &Environment, payload: &StandardLoggingPayload) -> Vec<String> {
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

// get_datadog_tags() carries only the five base tags, because the intake envelope has no call
#[must_use]
pub fn envelope_tags(environment: &Environment) -> Vec<String> {
    let team = TeamMetadata {
        user_api_key_team_alias: None,
        team_alias: None,
        user_api_key_team_id: None,
        team_id: None,
    };
    tags(environment, &[], &team)
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
