use crate::otel::model::metadata::REQUESTER_METADATA_PATH;
use crate::otel::model::metadata::request_identity::RequestIdentity;
use crate::otel::model::utils::coercion::OtelModelValue;
use serde_json::to_string;
use std::collections::BTreeMap;

pub const TEAM_ID: &str = "litellm.team.id";
pub const TEAM_ALIAS: &str = "litellm.team.alias";
pub const TEAM_METADATA: &str = "litellm.team.metadata";
pub const KEY_HASH: &str = "litellm.api_key.hash";
pub const END_USER: &str = "litellm.end_user.id";
pub const REQUEST_MODEL: &str = "gen_ai.request.model";
pub const PROVIDER_MODEL: &str = "litellm.provider.model";
pub const METADATA_PREFIX: &str = "litellm.metadata.";

pub const BAGGAGE_PROMOTED_KEYS: &[&str] = &[
    TEAM_ID,
    TEAM_ALIAS,
    TEAM_METADATA,
    KEY_HASH,
    REQUEST_MODEL,
    PROVIDER_MODEL,
];

#[must_use]
pub fn promoted_baggage(
    identity: &RequestIdentity,
    request_model: Option<&str>,
    promoted_keys: &[String],
    metadata_keys: &[String],
    team_metadata_keys: &[String],
) -> BTreeMap<String, String> {
    let candidates = [
        (TEAM_ID, identity.team_id.as_deref()),
        (TEAM_ALIAS, identity.team_alias.as_deref()),
        (KEY_HASH, identity.key_hash.as_deref()),
        (END_USER, identity.end_user.as_deref()),
        (REQUEST_MODEL, request_model),
        (PROVIDER_MODEL, identity.provider_model.as_deref()),
    ];
    let mut output: BTreeMap<String, String> = candidates
        .into_iter()
        .filter(|(key, value)| {
            promoted_keys.iter().any(|selected| selected == key)
                && value.is_some_and(|text| !text.is_empty())
        })
        .filter_map(|(key, value)| value.map(|text| (key.to_owned(), text.to_owned())))
        .collect();
    if promoted_keys.iter().any(|key| key == TEAM_METADATA)
        && let Some(encoded) =
            filtered_team_metadata_json(identity.team_metadata.as_ref(), team_metadata_keys)
    {
        output.insert(TEAM_METADATA.to_owned(), encoded);
    }
    output.extend(promoted_metadata(&identity.metadata, metadata_keys));
    output
}

#[must_use]
pub fn promoted_metadata(
    metadata: &BTreeMap<String, String>,
    metadata_keys: &[String],
) -> BTreeMap<String, String> {
    metadata_keys
        .iter()
        .filter_map(|key| {
            let value = metadata.get(key)?;
            (!value.is_empty()).then(|| {
                (
                    format!(
                        "{METADATA_PREFIX}{}",
                        key.strip_prefix(REQUESTER_METADATA_PATH).unwrap_or(key)
                    ),
                    value.clone(),
                )
            })
        })
        .collect()
}

#[must_use]
pub fn filtered_team_metadata_json(
    metadata_option: Option<&BTreeMap<String, OtelModelValue>>,
    allowed_keys: &[String],
) -> Option<String> {
    let metadata = metadata_option?;
    let filtered: BTreeMap<&str, &OtelModelValue> = allowed_keys
        .iter()
        .filter_map(|key| metadata.get(key).map(|value| (key.as_str(), value)))
        .collect();
    if filtered.is_empty() {
        return None;
    }
    let entries: Option<Vec<String>> = filtered
        .iter()
        .map(|(key, value)| {
            let quoted_key = to_string(key).ok()?;
            let encoded_value = encode_json_value(value)?;
            Some(format!("{quoted_key}: {encoded_value}"))
        })
        .collect();
    Some(format!("{{{}}}", entries?.join(", ")))
}

fn encode_json_value(value: &OtelModelValue) -> Option<String> {
    match value {
        OtelModelValue::Object(fields) => {
            let entries: Option<Vec<String>> = fields
                .iter()
                .map(|(key, item)| {
                    let quoted_key = to_string(key).ok()?;
                    let encoded_value = encode_json_value(item)?;
                    Some(format!("{quoted_key}: {encoded_value}"))
                })
                .collect();
            Some(format!("{{{}}}", entries?.join(", ")))
        }
        OtelModelValue::Array(items) => {
            let values: Option<Vec<String>> = items.iter().map(encode_json_value).collect();
            Some(format!("[{}]", values?.join(", ")))
        }
        OtelModelValue::Null(()) => Some("null".to_owned()),
        OtelModelValue::Bool(boolean) => to_string(boolean).ok(),
        OtelModelValue::Number(number) => to_string(number).ok(),
        OtelModelValue::String(text) => to_string(text).ok(),
    }
}

#[path = "baggage/tests.rs"]
mod tests;
