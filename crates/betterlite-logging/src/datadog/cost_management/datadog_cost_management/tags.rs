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
    for (key, value) in scalar_metadata(&payload.metadata) {
        if allow.iter().any(|allowed| allowed == key) {
            set_custom_tag(tags, key, &value);
        }
    }
    for (key, value) in nested_metadata(&payload.metadata) {
        if allow.iter().any(|allowed| allowed == key) {
            set_custom_tag(tags, key, &value);
        }
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
        collect_scalars(&mut found, entries);
    }
    found
}

fn collect_scalars<'entries>(
    found: &mut Vec<(&'entries str, String)>,
    entries: &'entries IndexMap<String, Json>,
) {
    for (key, value) in entries {
        if let Some(text) = scalar_text(value) {
            found.push((key.as_str(), text));
        }
    }
}

fn scalar_metadata(metadata: &StandardLoggingMetadata) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    push_strings(&mut found, metadata);
    push_numbers(&mut found, metadata);
    push_json_scalars(&mut found, metadata);
    if let Some(flag) = metadata.used_client_oauth_token {
        found.push((
            "used_client_oauth_token",
            if flag { "true" } else { "false" }.to_owned(),
        ));
    }
    found
}

fn push_strings(found: &mut Vec<(&'static str, String)>, metadata: &StandardLoggingMetadata) {
    push_key_strings(found, metadata);
    let strings = [
        (
            "requester_ip_address",
            metadata.requester_ip_address.as_deref(),
        ),
        ("user_agent", metadata.user_agent.as_deref()),
        (
            "cold_storage_object_key",
            metadata.cold_storage_object_key.as_deref(),
        ),
        ("team_alias", metadata.team_alias.as_deref()),
        ("team_id", metadata.team_id.as_deref()),
    ];
    collect_text(found, &strings);
}

fn push_key_strings(found: &mut Vec<(&'static str, String)>, metadata: &StandardLoggingMetadata) {
    push_identity_strings(found, metadata);
    let scope = [
        (
            "user_api_key_project_id",
            metadata.user_api_key_project_id.as_deref(),
        ),
        (
            "user_api_key_project_alias",
            metadata.user_api_key_project_alias.as_deref(),
        ),
        (
            "user_api_key_user_id",
            metadata.user_api_key_user_id.as_deref(),
        ),
        (
            "user_api_key_user_email",
            metadata.user_api_key_user_email.as_deref(),
        ),
        (
            "user_api_key_team_alias",
            metadata.user_api_key_team_alias.as_deref(),
        ),
        (
            "user_api_key_end_user_id",
            metadata.user_api_key_end_user_id.as_deref(),
        ),
        (
            "user_api_key_request_route",
            metadata.user_api_key_request_route.as_deref(),
        ),
    ];
    collect_text(found, &scope);
}

fn push_identity_strings(
    found: &mut Vec<(&'static str, String)>,
    metadata: &StandardLoggingMetadata,
) {
    let identity = [
        ("user_api_key_hash", metadata.user_api_key_hash.as_deref()),
        ("user_api_key_alias", metadata.user_api_key_alias.as_deref()),
        (
            "user_api_key_budget_reset_at",
            metadata.user_api_key_budget_reset_at.as_deref(),
        ),
        (
            "user_api_key_org_id",
            metadata.user_api_key_org_id.as_deref(),
        ),
        (
            "user_api_key_org_alias",
            metadata.user_api_key_org_alias.as_deref(),
        ),
        (
            "user_api_key_team_id",
            metadata.user_api_key_team_id.as_deref(),
        ),
    ];
    collect_text(found, &identity);
}

fn collect_text(found: &mut Vec<(&'static str, String)>, strings: &[(&'static str, Option<&str>)]) {
    for &(key, value) in strings {
        if let Some(text) = value {
            found.push((key, text.to_owned()));
        }
    }
}

fn push_numbers(found: &mut Vec<(&'static str, String)>, metadata: &StandardLoggingMetadata) {
    let numbers = [
        ("user_api_key_spend", metadata.user_api_key_spend.as_ref()),
        (
            "user_api_key_max_budget",
            metadata.user_api_key_max_budget.as_ref(),
        ),
        (
            "user_api_key_user_spend",
            metadata.user_api_key_user_spend.as_ref(),
        ),
        (
            "user_api_key_user_max_budget",
            metadata.user_api_key_user_max_budget.as_ref(),
        ),
        (
            "user_api_key_team_spend",
            metadata.user_api_key_team_spend.as_ref(),
        ),
        (
            "user_api_key_team_max_budget",
            metadata.user_api_key_team_max_budget.as_ref(),
        ),
    ];
    for (key, value) in numbers {
        if let Some(number) = value {
            found.push((key, number.0.to_string()));
        }
    }
}

fn push_json_scalars(found: &mut Vec<(&'static str, String)>, metadata: &StandardLoggingMetadata) {
    let values = [
        ("spend_logs_metadata", metadata.spend_logs_metadata.as_ref()),
        ("requester_metadata", metadata.requester_metadata.as_ref()),
        (
            "prompt_management_metadata",
            metadata.prompt_management_metadata.as_ref(),
        ),
        (
            "mcp_tool_call_metadata",
            metadata.mcp_tool_call_metadata.as_ref(),
        ),
        ("routing_decision", metadata.routing_decision.as_ref()),
        ("usage_object", metadata.usage_object.as_ref()),
    ];
    for (key, value) in values {
        if let Some(text) = value.and_then(scalar_text) {
            found.push((key, text));
        }
    }
}

fn scalar_text(value: &Json) -> Option<String> {
    match value {
        Json::Bool(set) => Some(if *set { "true" } else { "false" }.to_owned()),
        Json::Number(number) => Some(number.to_string()),
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}
