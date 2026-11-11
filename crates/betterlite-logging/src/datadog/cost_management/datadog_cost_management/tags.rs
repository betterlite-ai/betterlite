use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::datadog::tagging::{Environment, normalize_value};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::payload::records::StandardLoggingMetadata;

pub const RESERVED_TAG_KEYS: [&str; 10] = [
    "env",
    "service",
    "host",
    "pod_name",
    "provider",
    "model",
    "model_id",
    "team",
    "user",
    "model_group",
];

#[must_use]
pub fn extract_tags(
    environment: &Environment,
    cost_tag_keys: &[String],
    payload: &StandardLoggingPayload,
) -> IndexMap<String, String> {
    let mut tags = IndexMap::new();
    tags.insert("env".to_owned(), environment.env.clone());
    tags.insert("service".to_owned(), environment.service.clone());
    tags.insert("host".to_owned(), environment.hostname.clone());
    tags.insert("pod_name".to_owned(), environment.pod_name.clone());
    insert_raw(
        &mut tags,
        "provider",
        payload.custom_llm_provider.as_deref(),
    );
    insert_raw(&mut tags, "model", Some(payload.model.as_str()));
    insert_raw(&mut tags, "model_id", payload.model_id.as_deref());
    insert_user_team_group(&mut tags, payload);
    if !cost_tag_keys.is_empty() {
        apply_allowlist(&mut tags, cost_tag_keys, payload);
    }
    tags
}

fn insert_raw(tags: &mut IndexMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(text) = value {
        tags.insert(key.to_owned(), text.to_owned());
    }
}

fn insert_user_team_group(tags: &mut IndexMap<String, String>, payload: &StandardLoggingPayload) {
    let metadata = &payload.metadata;
    if let Some(alias) = metadata.user_api_key_alias.as_deref() {
        tags.insert("user".to_owned(), normalize_value(alias));
    }
    let team = metadata
        .user_api_key_team_alias
        .as_deref()
        .or(metadata.team_alias.as_deref())
        .or(metadata.user_api_key_team_id.as_deref())
        .or(metadata.team_id.as_deref());
    if let Some(name) = team {
        tags.insert("team".to_owned(), normalize_value(name));
    }
    if let Some(group) = payload.model_group.as_deref() {
        tags.insert("model_group".to_owned(), group.to_owned());
    }
}

fn apply_allowlist(
    tags: &mut IndexMap<String, String>,
    allow: &[String],
    payload: &StandardLoggingPayload,
) {
    for tag in &payload.request_tags {
        if let Json::Text(text) = tag {
            add_request_tag(tags, allow, text);
        }
    }
    let metadata = &payload.metadata;
    let decoded = match serde_json::to_string(metadata)
        .and_then(|text| serde_json::from_str::<Json>(&text))
    {
        Ok(decoded) => decoded,
        Err(error) => {
            tracing::error!(
                %error,
                "Datadog Cost Management: the request metadata could not be read; allowlisted metadata tags are skipped"
            );
            return;
        }
    };
    if let Json::Object(fields) = decoded {
        for (key, value) in &fields {
            add_metadata_tag(tags, allow, key, value);
        }
    }
    for (key, value) in nested_metadata(metadata) {
        if allow.iter().any(|allowed| allowed == key) {
            set_custom_tag(tags, key, &value);
        }
    }
}

fn add_metadata_tag(
    tags: &mut IndexMap<String, String>,
    allow: &[String],
    key: &str,
    value: &Json,
) {
    if allow.iter().any(|allowed| allowed == key)
        && let Some(text) = json_text(value)
    {
        set_custom_tag(tags, key, &text);
    }
}

fn add_request_tag(tags: &mut IndexMap<String, String>, allow: &[String], text: &str) {
    let Some((key, value)) = text.split_once(':') else {
        return;
    };
    if allow.iter().any(|allowed| allowed == key) && !value.is_empty() {
        set_custom_tag(tags, key, value);
    }
}

fn set_custom_tag(tags: &mut IndexMap<String, String>, key: &str, value: &str) {
    if RESERVED_TAG_KEYS.contains(&key) {
        tracing::debug!(
            key,
            value,
            "Datadog Cost Management: dropping a user-supplied tag for a reserved trusted key"
        );
        return;
    }
    tags.insert(key.to_owned(), normalize_value(value));
}

fn nested_metadata(metadata: &StandardLoggingMetadata) -> Vec<(&str, String)> {
    let mut found = Vec::new();
    let fields = [
        metadata.spend_logs_metadata.as_ref(),
        metadata.requester_metadata.as_ref(),
    ];
    for value in fields {
        let Some(Json::Object(entries)) = value else {
            continue;
        };
        for (key, nested) in entries {
            if let Some(text) = json_text(nested) {
                found.push((key.as_str(), text));
            }
        }
    }
    found
}

fn json_text(value: &Json) -> Option<String> {
    match value {
        Json::Bool(set) => Some(if *set { "true" } else { "false" }.to_owned()),
        Json::Number(number) => Some(number.to_string()),
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}
