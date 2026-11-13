use indexmap::IndexMap;
use serde_json::Value;

// the walk follows the caller's option order, which decides which token limit wins
pub type Options = IndexMap<String, Value>;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 7] = [
    "stream",
    "temperature",
    "max_tokens",
    "max_completion_tokens",
    "top_p",
    "stop",
    "n",
];

const ALLOW_ZERO_TEMPERATURE: &str = "aws_sagemaker_allow_zero_temp";
const ZERO_TEMPERATURE: f64 = 0.01;
const MAX_NEW_TOKENS: &str = "max_new_tokens";
const BEST_OF: &str = "best_of";

pub fn map_openai_params(non_default_params: &mut Options, optional_params: &mut Options) {
    let allow_zero = non_default_params
        .get(ALLOW_ZERO_TEMPERATURE)
        .is_some_and(is_truthy);
    for (name, value) in non_default_params.iter() {
        match name.as_str() {
            "temperature" => {
                // TGI rejects a zero temperature; the flag is the caller's way to keep it
                let temperature = if is_zero(value) && !allow_zero {
                    Value::from(ZERO_TEMPERATURE)
                } else {
                    value.clone()
                };
                optional_params.insert(name.clone(), temperature);
            }
            "top_p" | "stream" | "stop" => {
                optional_params.insert(name.clone(), value.clone());
            }
            "n" => {
                optional_params.insert(String::from(BEST_OF), value.clone());
                optional_params.insert(String::from("do_sample"), Value::Bool(true));
            }
            "max_tokens" => {
                // TGI rejects max_new_tokens 0 as well, and only max_tokens gets the rewrite
                let limit = if is_zero(value) {
                    Value::from(1)
                } else {
                    value.clone()
                };
                optional_params.insert(String::from(MAX_NEW_TOKENS), limit);
            }
            "max_completion_tokens" => {
                optional_params.insert(String::from(MAX_NEW_TOKENS), value.clone());
            }
            _ => {}
        }
    }
    non_default_params.remove(ALLOW_ZERO_TEMPERATURE);
}

// the option map is LiteLLM's free-form dict, where the flag is read for its truth
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

fn is_zero(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64() == Some(0.0_f64),
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            false
        }
    }
}

mod tests;
