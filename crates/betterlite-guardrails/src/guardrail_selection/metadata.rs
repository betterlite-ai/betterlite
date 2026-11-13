use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::GuardrailSelectionError;

type Data = BTreeMap<String, GuardrailValue>;

// LiteLLM's _get_admin_metadata: key values lie over team values
pub(super) fn admin_metadata(data: &Data) -> BTreeMap<&str, &GuardrailValue> {
    let mut team = None;
    let mut key = None;
    for location in ["metadata", "litellm_metadata"] {
        let Some(GuardrailValue::Object(metadata)) = data.get(location) else {
            continue;
        };
        if let Some(GuardrailValue::Object(found)) = metadata.get("user_api_key_team_metadata")
            && !found.is_empty()
        {
            team = Some(found);
        }
        if let Some(GuardrailValue::Object(found)) = metadata.get("user_api_key_metadata")
            && !found.is_empty()
        {
            key = Some(found);
        }
    }
    let mut merged = BTreeMap::new();
    for layer in [team, key].into_iter().flatten() {
        for (name, value) in layer {
            merged.insert(name.as_str(), value);
        }
    }
    merged
}

pub(super) fn global_guardrails_disabled(data: &Data) -> bool {
    match admin_metadata(data).get("disable_global_guardrails") {
        Some(GuardrailValue::Boolean(disabled)) => *disabled,
        _ => false,
    }
}

pub(super) fn opted_out_global_guardrails(data: &Data) -> Vec<&str> {
    let admin = admin_metadata(data);
    let Some(GuardrailValue::Array(entries)) = admin.get("opted_out_global_guardrails").copied()
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            GuardrailValue::Text(name) => Some(name.as_str()),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_) => None,
        })
        .collect()
}

// LiteLLM's get_guardrail_from_metadata
pub(super) fn requested_guardrails(
    data: &Data,
) -> Result<&[GuardrailValue], GuardrailSelectionError> {
    let deciding = data.get("guardrails").or_else(|| {
        ["metadata", "litellm_metadata"]
            .into_iter()
            .find_map(|location| match data.get(location) {
                Some(GuardrailValue::Object(metadata)) => metadata.get("guardrails"),
                _ => None,
            })
    });
    match deciding {
        None | Some(GuardrailValue::Null) => Ok(&[]),
        Some(GuardrailValue::Array(entries)) => Ok(entries),
        Some(_) => Err(GuardrailSelectionError::RequestedNotList),
    }
}
