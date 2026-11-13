use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// LiteLLM reads litellm_metadata before metadata here, the reverse of the route lookup
const PERMISSION_ID_BUCKETS: [&str; 2] = ["litellm_metadata", "metadata"];

type PermissionIds = (Option<String>, Option<String>);

const fn object_value(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    match value {
        GuardrailValue::Object(fields) => Some(fields),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => None,
    }
}

// LiteLLM prints str(id).strip() for a truthy id, so a number id becomes its text
fn permission_id(value: Option<&GuardrailValue>) -> Option<String> {
    match value {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.trim().to_owned()),
        Some(GuardrailValue::SignedInteger(number)) => Some(number.to_string()),
        Some(GuardrailValue::UnsignedInteger(number)) => Some(number.to_string()),
        None
        | Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        ) => None,
    }
}

fn read_ids(key: Option<&GuardrailValue>, team: Option<&GuardrailValue>) -> Option<PermissionIds> {
    let named = !matches!(key, None | Some(GuardrailValue::Null))
        || !matches!(team, None | Some(GuardrailValue::Null));
    if !named {
        return None;
    }
    Some((permission_id(key), permission_id(team)))
}

fn bucket_ids(bucket: &BTreeMap<String, GuardrailValue>) -> Option<PermissionIds> {
    if let Some(auth) = bucket.get("user_api_key_auth").and_then(object_value) {
        let nested = read_ids(
            auth.get("object_permission_id"),
            auth.get("team_object_permission_id"),
        );
        if nested.is_some() {
            return nested;
        }
    }
    read_ids(
        bucket.get("user_api_key_object_permission_id"),
        bucket.get("user_api_key_team_object_permission_id"),
    )
}

#[must_use]
pub(super) fn permission_ids_from_data(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> PermissionIds {
    for name in PERMISSION_ID_BUCKETS {
        let Some(GuardrailValue::Object(bucket)) = request_data.get(name) else {
            continue;
        };
        if let Some(ids) = bucket_ids(bucket) {
            return ids;
        }
    }
    (None, None)
}

#[must_use]
pub(super) fn request_route_from_data(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Option<String> {
    if let Some(GuardrailValue::Text(route)) = request_data.get("user_api_key_request_route")
        && !route.is_empty()
    {
        return Some(route.clone());
    }
    let metadata = request_data
        .get("metadata")
        .and_then(object_value)
        .filter(|fields| !fields.is_empty());
    // an empty litellm_metadata has no route either, so it needs no empty check of its own
    let litellm_metadata = request_data.get("litellm_metadata").and_then(object_value);
    let bucket = metadata.or(litellm_metadata)?;
    let Some(GuardrailValue::Text(route)) = bucket.get("user_api_key_request_route") else {
        return None;
    };
    Some(route.clone())
}
