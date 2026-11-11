use crate::datadog::tagging::normalize_value;
use crate::fields::payload::StandardLoggingPayload;

use super::settings::MetricsEnvironment;

#[must_use]
pub fn extract_tags(
    environment: &MetricsEnvironment,
    payload: &StandardLoggingPayload,
    status_code: Option<&str>,
) -> Vec<String> {
    let mut tags = vec![
        format!("env:{}", environment.env),
        format!("service:{}", environment.service),
        format!("version:{}", environment.version),
        format!("HOSTNAME:{}", environment.hostname),
        format!("POD_NAME:{}", environment.pod_name),
    ];
    // LiteLLM adds each of these only for a non-empty value, so an empty one names no series
    if let Some(provider) = payload
        .custom_llm_provider
        .as_deref()
        .filter(|provider| !provider.is_empty())
    {
        tags.push(format!("provider:{provider}"));
    }
    if !payload.model.is_empty() {
        tags.push(format!("model_name:{}", payload.model));
    }
    if let Some(model_group) = payload
        .model_group
        .as_deref()
        .filter(|group| !group.is_empty())
    {
        tags.push(format!("model_group:{model_group}"));
    }
    if let Some(code) = status_code {
        tags.push(format!("status_code:{code}"));
    }
    if let Some(team) = team_tag(payload) {
        tags.push(format!("team:{}", normalize_value(team)));
    }
    tags
}

// LiteLLM chains the four metadata keys with `or`, so an empty value falls through to the next one
fn team_tag(payload: &StandardLoggingPayload) -> Option<&str> {
    let candidates = [
        payload.metadata.user_api_key_team_alias.as_deref(),
        payload.metadata.team_alias.as_deref(),
        payload.metadata.user_api_key_team_id.as_deref(),
        payload.metadata.team_id.as_deref(),
    ];
    candidates
        .iter()
        .flatten()
        .copied()
        .find(|name| !name.is_empty())
}
