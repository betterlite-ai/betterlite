use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};

use crate::straiker_webhook::types::StraikerWebhookIdentity;

const APPLICATION_METADATA_KEYS: [&str; 2] = ["agent_id", "app_name"];
const USER_API_PREFIX: &str = "user_api";
const SESSION_METADATA_KEY: &str = "session_id";

#[must_use]
pub fn merged_metadata(request_data: &Object) -> Object {
    let mut merged = request_data
        .get("metadata")
        .and_then(Json::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(litellm_metadata) = request_data
        .get("litellm_metadata")
        .and_then(Json::as_object)
    {
        merged.extend(
            litellm_metadata
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    merged
}

#[must_use]
pub fn webhook_metadata(
    request_data: &Object,
    defaults: &BTreeMap<String, String>,
) -> Option<Object> {
    let mut out = Object::new();
    let fields = request_data.get("metadata").and_then(Json::as_object);
    for (key, value) in fields.into_iter().flatten() {
        let filtered = APPLICATION_METADATA_KEYS.contains(&key.as_str())
            || key.starts_with(USER_API_PREFIX)
            || key == SESSION_METADATA_KEY;
        if filtered || !matches!(value, Json::Text(_) | Json::Number(_) | Json::Bool(_)) {
            continue;
        }
        out.insert(key.clone(), value.clone());
    }
    out.extend(
        defaults
            .iter()
            .map(|(key, value)| (key.clone(), Json::Text(value.clone()))),
    );
    (!out.is_empty()).then_some(out)
}

#[must_use]
pub fn extract_identity(request_data: &Object) -> StraikerWebhookIdentity {
    let metadata = merged_metadata(request_data);
    let text = |key: &str| {
        metadata
            .get(key)
            .and_then(Json::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    StraikerWebhookIdentity {
        litellm_key: text("user_api_key_alias")
            .or_else(|| text("user_api_key_hash"))
            .or_else(|| text("user_api_key_token")),
        litellm_team: text("user_api_key_team_alias").or_else(|| text("user_api_key_team_id")),
        litellm_user_id: text("user_api_key_user_id"),
        litellm_user_email: text("user_api_key_user_email"),
        litellm_org_id: text("user_api_key_org_id"),
        end_user_id: text("user_api_key_end_user_id"),
    }
}

mod tests;
