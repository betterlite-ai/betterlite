use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use super::records::GuardrailRecord;

// what LiteLLM shows when a record leaves its provider or kind out
pub const UNKNOWN_PROVIDER: &str = "Unknown";
pub const DEFAULT_GUARDRAIL_TYPE: &str = "Guardrail";

static EMPTY_PARAMS: OpaqueParams = OpaqueParams(BTreeMap::new());

#[must_use]
pub fn field_str(mapping: &OpaqueParams, key: &str, default: &str) -> String {
    match mapping.0.get(key) {
        Some(GuardrailValue::Text(text)) => text.clone(),
        Some(GuardrailValue::SignedInteger(number)) => number.to_string(),
        Some(GuardrailValue::UnsignedInteger(number)) => number.to_string(),
        Some(GuardrailValue::Decimal(number)) => python_float_text(*number),
        Some(_) | None => default.to_owned(),
    }
}

// Python's str(float) writes the exponent with a sign and two digits at least; Rust's Debug does neither
fn python_float_text(number: f64) -> String {
    let text = format!("{number:?}");
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return text;
    };
    format!("{mantissa}e{exponent:+03}")
}

#[must_use]
pub fn guardrail_provider(record: &GuardrailRecord) -> String {
    record
        .litellm_params
        .guardrail
        .as_deref()
        .unwrap_or(UNKNOWN_PROVIDER)
        .to_owned()
}

#[must_use]
pub fn guardrail_type(record: &GuardrailRecord) -> String {
    field_str(
        record.guardrail_info.as_ref().unwrap_or(&EMPTY_PARAMS),
        "type",
        DEFAULT_GUARDRAIL_TYPE,
    )
}

#[must_use]
pub fn guardrail_attrs(record: &GuardrailRecord) -> (String, String) {
    let id = record.guardrail_id.clone().unwrap_or_default();
    let name = match record.guardrail_name.as_deref() {
        Some(name) if !name.is_empty() => name.to_owned(),
        Some(_) | None => id.clone(),
    };
    (id, name)
}

mod tests;
