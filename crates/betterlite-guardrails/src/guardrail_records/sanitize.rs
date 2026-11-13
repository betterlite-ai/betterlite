use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_redact::structured::mask_credentials_in_payload;

use super::error::GuardrailRecordError;
use crate::xecguard_primitives::redaction::redact_match_and_regex_keys;

pub(super) const MAX_NULL_DROP_DEPTH: usize = 20;
const SECRET_FIELDS_KEY: &str = "secret_fields";

// LiteLLM's filter_exceptions_from_params: the root is level 0 and a value at level 20 is kept whole
pub(super) fn drop_nulls(value: GuardrailValue, levels_left: usize) -> GuardrailValue {
    if levels_left == 0 {
        return value;
    }
    let next_levels = levels_left.saturating_sub(1);
    match value {
        GuardrailValue::Object(fields) => GuardrailValue::Object(
            fields
                .into_iter()
                .filter(|(_key, field)| *field != GuardrailValue::Null)
                .map(|(key, field)| (key, drop_nulls(field, next_levels)))
                .collect(),
        ),
        GuardrailValue::Array(items) => GuardrailValue::Array(
            items
                .into_iter()
                .filter(|item| *item != GuardrailValue::Null)
                .map(|item| drop_nulls(item, next_levels))
                .collect(),
        ),
        scalar @ (GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)) => scalar,
    }
}

pub fn sanitize_guardrail_response(
    response: GuardrailValue,
) -> Result<GuardrailValue, GuardrailRecordError> {
    let mut cleaned = drop_nulls(response, MAX_NULL_DROP_DEPTH);
    match &mut cleaned {
        GuardrailValue::Object(fields) => drop(fields.remove(SECRET_FIELDS_KEY)),
        GuardrailValue::Array(items) => {
            for item in items {
                if let GuardrailValue::Object(fields) = item {
                    drop(fields.remove(SECRET_FIELDS_KEY));
                }
            }
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => {}
    }
    let redacted = redact_match_and_regex_keys(&cleaned);
    let serialized =
        serde_json::to_value(&redacted).map_err(|source| GuardrailRecordError::Json { source })?;
    serde_json::from_value(mask_credentials_in_payload(&serialized))
        .map_err(|source| GuardrailRecordError::Json { source })
}
