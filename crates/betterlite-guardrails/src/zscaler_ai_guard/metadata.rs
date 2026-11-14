use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::ZscalerAiGuardError;
use super::printed::str_text;

const METADATA: &str = "metadata";
const LITELLM_METADATA: &str = "litellm_metadata";
const USER_API_KEY_METADATA: &str = "user_api_key_metadata";
const TEAM_METADATA: &str = "team_metadata";
const ZGUARD_POLICY_ID: &str = "zguard_policy_id";
const ALIAS: &str = "user_api_key_alias";
const KEY_ALIAS: &str = "user_api_key_key_alias";

type Section<'data> = Option<&'data BTreeMap<String, GuardrailValue>>;

// LiteLLM's `or {}` only guards an absent or null section; any other non-object is an error that names the key
fn section<'data>(
    data: &'data BTreeMap<String, GuardrailValue>,
    name: &'static str,
) -> Result<Section<'data>, ZscalerAiGuardError> {
    match data.get(name) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(fields)) => Ok(Some(fields)),
        Some(_wrong) => Err(ZscalerAiGuardError::MetadataShape { name }),
    }
}

// LiteLLM checks the key's presence, so a null zguard_policy_id still wins over the configured id
pub fn selected_policy_id(
    request_data: &BTreeMap<String, GuardrailValue>,
    configured: i64,
) -> Result<GuardrailValue, ZscalerAiGuardError> {
    let metadata = section(request_data, METADATA)?;
    let user_key = metadata
        .map(|fields| section(fields, USER_API_KEY_METADATA))
        .transpose()?
        .flatten();
    let team = metadata
        .map(|fields| section(fields, TEAM_METADATA))
        .transpose()?
        .flatten();
    let selected = metadata
        .and_then(|fields| fields.get(ZGUARD_POLICY_ID))
        .or_else(|| user_key.and_then(|fields| fields.get(ZGUARD_POLICY_ID)))
        .or_else(|| team.and_then(|fields| fields.get(ZGUARD_POLICY_ID)));
    Ok(selected
        .cloned()
        .unwrap_or(GuardrailValue::SignedInteger(configured)))
}

fn metadata_value(
    request_data: &BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Result<Option<String>, ZscalerAiGuardError> {
    let litellm_metadata = section(request_data, LITELLM_METADATA)?;
    if let Some(value) = litellm_metadata
        .and_then(|fields| fields.get(key))
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        return Ok(Some(str_text(value).trim().to_owned()));
    }
    // transform_user_api_key_dict_to_metadata prefixes key_alias with user_api_key_
    if key == ALIAS
        && let Some(value) = litellm_metadata
            .and_then(|fields| fields.get(KEY_ALIAS))
            .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        return Ok(Some(str_text(value).trim().to_owned()));
    }
    let metadata = section(request_data, METADATA)?;
    if let Some(value) = metadata
        .and_then(|fields| fields.get(key))
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        return Ok(Some(str_text(value).trim().to_owned()));
    }
    Ok(None)
}

// LiteLLM's `_resolve_metadata_value(...) or "N/A"`
pub fn header_value(
    request_data: &BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Result<String, ZscalerAiGuardError> {
    Ok(match metadata_value(request_data, key)? {
        Some(text) if !text.is_empty() => text,
        _ => "N/A".to_owned(),
    })
}

// policy_id is only written when it is not null and at least 1; LiteLLM compares the raw value, so a non-number fails
pub fn policy_id_wire(
    value: &GuardrailValue,
) -> Result<Option<GuardrailValue>, ZscalerAiGuardError> {
    match value {
        GuardrailValue::Null => Ok(None),
        GuardrailValue::SignedInteger(id) if *id >= 1 => Ok(Some(value.clone())),
        GuardrailValue::UnsignedInteger(id) if *id >= 1 => Ok(Some(value.clone())),
        GuardrailValue::Decimal(id) if *id >= 1.0 => Ok(Some(value.clone())),
        GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_) => Ok(None),
        GuardrailValue::Text(_) | GuardrailValue::Array(_) | GuardrailValue::Object(_) => {
            Err(ZscalerAiGuardError::PolicyIdType)
        }
    }
}
