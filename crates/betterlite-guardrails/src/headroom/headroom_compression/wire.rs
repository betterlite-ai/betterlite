use serde_json::Number;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::primitives::json_value::JsonValue;

pub(crate) fn guardrail_value(value: &JsonValue) -> GuardrailValue {
    match value {
        JsonValue::Null => GuardrailValue::Null,
        JsonValue::Bool(flag) => GuardrailValue::Boolean(*flag),
        JsonValue::Number(number) => number_value(number),
        JsonValue::String(text) => GuardrailValue::Text(text.clone()),
        JsonValue::Array(items) => {
            GuardrailValue::Array(items.iter().map(guardrail_value).collect())
        }
        JsonValue::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), guardrail_value(field)))
                .collect(),
        ),
    }
}

fn number_value(number: &Number) -> GuardrailValue {
    if let Some(signed) = number.as_i64() {
        return GuardrailValue::SignedInteger(signed);
    }
    if let Some(unsigned) = number.as_u64() {
        return GuardrailValue::UnsignedInteger(unsigned);
    }
    match number.as_f64() {
        Some(decimal) => GuardrailValue::Decimal(decimal),
        None => GuardrailValue::Null,
    }
}
