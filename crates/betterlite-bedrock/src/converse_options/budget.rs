use serde_json::Number;

use crate::model_routes::{ParamMap, ParamValue};

const BUDGET_FIELD: &str = "budget_tokens";
const THINKING_FIELD: &str = "thinking";

// LiteLLM's isinstance(budget, int): JSON booleans count as integers, floats and text do not.
pub fn clamp_thinking_budget_tokens(optional_params: &mut ParamMap, minimum: i64) {
    let Some(ParamValue::Mapping(thinking)) = optional_params.get_mut(THINKING_FIELD) else {
        return;
    };
    let Some(budget) = thinking.get(BUDGET_FIELD) else {
        return;
    };
    let got = match budget {
        ParamValue::Bool(flag) => i64::from(*flag),
        ParamValue::Number(number) => {
            let Some(value) = number.as_i64() else {
                return;
            };
            value
        }
        ParamValue::Null | ParamValue::Text(_) | ParamValue::List(_) | ParamValue::Mapping(_) => {
            return;
        }
    };
    if got >= minimum {
        return;
    }
    tracing::debug!(
        "Bedrock requires thinking.budget_tokens >= {minimum}, got {got}. Clamping to minimum."
    );
    thinking.insert(
        BUDGET_FIELD.to_owned(),
        ParamValue::Number(Number::from(minimum)),
    );
}
