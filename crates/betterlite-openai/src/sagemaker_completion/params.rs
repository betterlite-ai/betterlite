use betterlite_prompts::arguments::json::Json;
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
        .and_then(|flag| serde_json::from_value::<Json>(flag.clone()).ok())
        .is_some_and(|flag| flag.is_truthy());
    for (name, value) in non_default_params.iter() {
        match name.as_str() {
            "temperature" => {
                // TGI rejects a zero temperature; the flag is the caller's way to keep it
                let temperature = if value.as_f64() == Some(0.0_f64) && !allow_zero {
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
                let limit = if value.as_f64() == Some(0.0_f64) {
                    Value::from(1_u64)
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
    non_default_params.shift_remove(ALLOW_ZERO_TEMPERATURE);
}

mod tests;
