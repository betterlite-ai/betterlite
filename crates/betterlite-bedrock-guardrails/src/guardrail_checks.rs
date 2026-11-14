use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_translation::BedrockChecksConfigModel;
use serde::Serialize;

use crate::bedrock_transport::BedrockGuardrailError;
use crate::constants::BEDROCK_CHECKS_KNOWN_KEYS;

#[derive(Debug, Clone)]
pub enum ChecksInput {
    Model(BedrockChecksConfigModel),
    Mapping(BTreeMap<String, GuardrailValue>),
}

pub fn normalize_checks(
    checks: Option<ChecksInput>,
) -> Result<Option<BTreeMap<String, GuardrailValue>>, BedrockGuardrailError> {
    let Some(input) = checks else {
        return Ok(None);
    };
    let raw = match input {
        ChecksInput::Model(model) => model_checks(model)?,
        ChecksInput::Mapping(mapping) => mapping,
    };
    let unknown: Vec<String> = raw
        .keys()
        .filter(|key| !BEDROCK_CHECKS_KNOWN_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        tracing::warn!(
            "BedrockGuardrail: unrecognized check key(s) {unknown:?} will be ignored; recognized keys will still be used for InvokeGuardrailChecks. Known keys: [\"contentFilter\", \"promptAttack\", \"sensitiveInformation\"]."
        );
    }
    let mut cleaned = BTreeMap::new();
    for (key, value) in &raw {
        if BEDROCK_CHECKS_KNOWN_KEYS.contains(&key.as_str())
            && !matches!(value, GuardrailValue::Null)
        {
            cleaned.insert(key.clone(), value.clone());
        }
    }
    if cleaned.is_empty() && !raw.is_empty() {
        return Err(BedrockGuardrailError::NoUsableChecks {
            keys: raw.keys().cloned().collect(),
        });
    }
    Ok((!cleaned.is_empty()).then_some(cleaned))
}

fn model_checks(
    model: BedrockChecksConfigModel,
) -> Result<BTreeMap<String, GuardrailValue>, BedrockGuardrailError> {
    let mut checks = BTreeMap::new();
    if let Some(content_filter) = model.content_filter {
        insert_check(&mut checks, "contentFilter", &content_filter)?;
    }
    if let Some(prompt_attack) = model.prompt_attack {
        insert_check(&mut checks, "promptAttack", &prompt_attack)?;
    }
    if let Some(sensitive_information) = model.sensitive_information {
        insert_check(&mut checks, "sensitiveInformation", &sensitive_information)?;
    }
    Ok(checks)
}

fn insert_check<T>(
    checks: &mut BTreeMap<String, GuardrailValue>,
    key: &str,
    value: &T,
) -> Result<(), BedrockGuardrailError>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(BedrockGuardrailError::Encode)?;
    let decoded = serde_json::from_value(encoded).map_err(BedrockGuardrailError::Encode)?;
    checks.insert(key.to_owned(), decoded);
    Ok(())
}
