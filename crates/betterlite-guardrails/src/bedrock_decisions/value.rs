use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::decisions::Json;
use serde_json::Number;

// model_armor's twin is private to its module, so the request record's GuardrailValue copy is rebuilt here
#[must_use]
pub fn json_to_value(value: &Json) -> GuardrailValue {
    match value {
        Json::Null => GuardrailValue::Null,
        Json::Bool(flag) => GuardrailValue::Boolean(*flag),
        Json::Number(number) => number_to_value(number),
        Json::Text(text) => GuardrailValue::Text(text.clone()),
        Json::List(items) => GuardrailValue::Array(items.iter().map(json_to_value).collect()),
        Json::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), json_to_value(item)))
                .collect(),
        ),
    }
}

fn number_to_value(number: &Number) -> GuardrailValue {
    if let Some(value) = number.as_i64() {
        return GuardrailValue::SignedInteger(value);
    }
    if let Some(value) = number.as_u64() {
        return GuardrailValue::UnsignedInteger(value);
    }
    if let Some(value) = number.as_f64() {
        return GuardrailValue::Decimal(value);
    }
    GuardrailValue::Text(number.to_string())
}
