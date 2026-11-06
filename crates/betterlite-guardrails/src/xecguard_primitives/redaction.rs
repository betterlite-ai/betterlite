use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_redact::structured::mask_credentials_in_payload;

use super::error::XecGuardError;
use super::wire::ScanResult;

const REDACTED_VALUE: &str = "[REDACTED]";
const SECRET_FIELDS_KEY: &str = "secret_fields";
const REDACTED_KEYS: [&str; 2] = ["match", "regex"];

#[must_use]
pub fn redact_match_and_regex_keys(payload: &GuardrailValue) -> GuardrailValue {
    let mut redacted = payload.clone();
    let mut stack = vec![&mut redacted];
    while let Some(node) = stack.pop() {
        redact_node(node, &mut stack);
    }
    redacted
}

fn redact_node<'tree>(node: &'tree mut GuardrailValue, stack: &mut Vec<&'tree mut GuardrailValue>) {
    match node {
        GuardrailValue::Object(fields) => {
            for (key, value) in fields.iter_mut() {
                if REDACTED_KEYS.contains(&key.as_str()) {
                    *value = GuardrailValue::Text(REDACTED_VALUE.to_owned());
                } else {
                    stack.push(value);
                }
            }
        }
        GuardrailValue::Array(items) => stack.extend(items.iter_mut()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => {}
    }
}

pub fn sanitize_scan_result(
    scan_result: &ScanResult,
) -> Result<BTreeMap<String, GuardrailValue>, XecGuardError> {
    let mut fields = scan_result.fields();
    drop(fields.remove(SECRET_FIELDS_KEY));
    let redacted = redact_match_and_regex_keys(&GuardrailValue::Object(fields));
    let serialized = serde_json::to_value(&redacted)
        .map_err(|source| XecGuardError::PayloadEncoding { source })?;
    let masked = mask_credentials_in_payload(&serialized);
    serde_json::from_value(masked).map_err(|source| XecGuardError::PayloadEncoding { source })
}

mod tests;
