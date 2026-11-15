use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::RequestData;
use super::text_value;

// LiteLLM reads these four buckets in this order, and skips an empty one
const BUCKETS: [&str; 2] = ["litellm_metadata", "metadata"];
const FIELDS: [&str; 7] = [
    "user_api_key_alias",
    "user_api_key_user_id",
    "user_api_key_user_email",
    "user_api_key_org_id",
    "user_api_key_org_alias",
    "user_api_key_team_id",
    "user_api_key_team_alias",
];
const ROLE_FIELD: &str = "user_api_key_user_role";
const AUTH_FIELD: &str = "user_api_key_auth";
const ROLE_SOURCE_FIELD: &str = "user_role";

// request_data carries betterlite's own typed state, so a value of another type reads as absent
pub(super) fn build(request_data: &RequestData) -> Option<BTreeMap<String, String>> {
    let buckets = buckets(request_data);
    let mut metadata = BTreeMap::new();
    for field in FIELDS {
        if let Some(value) = first_value(&buckets, field) {
            metadata.insert(field.to_owned(), value.to_owned());
        }
    }
    if let Some(role) = user_role(&buckets) {
        metadata.insert(ROLE_FIELD.to_owned(), role);
    }
    if metadata.is_empty() {
        return None;
    }
    Some(metadata)
}

fn buckets(request_data: &RequestData) -> Vec<&BTreeMap<String, GuardrailValue>> {
    let mut found = Vec::new();
    collect_buckets(request_data, &mut found);
    if let Some(GuardrailValue::Object(params)) = request_data.get("litellm_params") {
        collect_buckets(params, &mut found);
    }
    found
}

fn collect_buckets<'data>(
    source: &'data BTreeMap<String, GuardrailValue>,
    found: &mut Vec<&'data BTreeMap<String, GuardrailValue>>,
) {
    for name in BUCKETS {
        if let Some(GuardrailValue::Object(bucket)) = source.get(name)
            && !bucket.is_empty()
        {
            found.push(bucket);
        }
    }
}

fn first_value<'data>(
    buckets: &[&'data BTreeMap<String, GuardrailValue>],
    field: &str,
) -> Option<&'data str> {
    for bucket in buckets {
        if let Some(value) = bucket
            .get(field)
            .and_then(text_value)
            .filter(|text| !text.is_empty())
        {
            return Some(value);
        }
    }
    None
}

fn user_role(buckets: &[&BTreeMap<String, GuardrailValue>]) -> Option<String> {
    for bucket in buckets {
        if let Some(GuardrailValue::Object(auth)) = bucket.get(AUTH_FIELD)
            && let Some(role) = auth
                .get(ROLE_SOURCE_FIELD)
                .and_then(text_value)
                .filter(|text| !text.is_empty())
        {
            return Some(role.to_owned());
        }
    }
    None
}
