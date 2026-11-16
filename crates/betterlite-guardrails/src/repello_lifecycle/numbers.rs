use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::repello_content::wire::RepelloAIAnalyzeResponse;

// serde_json's arbitrary_precision hands an untagged GuardrailValue every float as this one-entry map
const PRIVATE_NUMBER_KEY: &str = "$serde_json::private::Number";

pub(super) fn repair(response: &mut RepelloAIAnalyzeResponse) {
    for policy in response.policies_violated.iter_mut().flatten() {
        let Some(details) = policy.details.as_mut() else {
            continue;
        };
        for value in details.values_mut() {
            repair_value(value);
        }
    }
    for applied in response.policies_applied.iter_mut().flatten() {
        for value in applied.values_mut() {
            repair_value(value);
        }
    }
}

fn repair_value(value: &mut GuardrailValue) {
    match value {
        GuardrailValue::Object(fields) => {
            if fields.len() == 1
                && let Some(GuardrailValue::Text(text)) = fields.get(PRIVATE_NUMBER_KEY)
                && let Some(number) = number_from_text(text)
            {
                *value = number;
                return;
            }
            for child in fields.values_mut() {
                repair_value(child);
            }
        }
        GuardrailValue::Array(items) => {
            for item in items {
                repair_value(item);
            }
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_) => {}
    }
}

fn number_from_text(text: &str) -> Option<GuardrailValue> {
    if let Ok(signed) = text.parse::<i64>() {
        return Some(GuardrailValue::SignedInteger(signed));
    }
    if let Ok(unsigned) = text.parse::<u64>() {
        return Some(GuardrailValue::UnsignedInteger(unsigned));
    }
    text.parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .map(GuardrailValue::Decimal)
}
