use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::settings::RequestData;

// GuardrailValue is JSON already; a non-finite float becomes null, as serde_json's Value does for NaN
#[must_use]
pub fn payload_json(data: &RequestData) -> Json {
    Json::Object(
        data.iter()
            .map(|(key, value)| (key.clone(), value_json(value)))
            .collect(),
    )
}

fn value_json(value: &GuardrailValue) -> Json {
    match value {
        GuardrailValue::Null => Json::Null,
        GuardrailValue::Boolean(flag) => Json::Bool(*flag),
        GuardrailValue::SignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::UnsignedInteger(number) => Json::Number(Number::from(*number)),
        GuardrailValue::Decimal(number) => {
            Number::from_f64(*number).map_or(Json::Null, Json::Number)
        }
        GuardrailValue::Text(text) => Json::Text(text.clone()),
        GuardrailValue::Array(items) => Json::List(items.iter().map(value_json).collect()),
        GuardrailValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(field, nested)| (field.clone(), value_json(nested)))
                .collect(),
        ),
    }
}
