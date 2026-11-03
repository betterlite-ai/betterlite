use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LLMRequestParams {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub max_tokens: Option<i64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub stop_sequences: Option<Vec<String>>,
    pub seed: Option<i64>,
}

impl LLMRequestParams {
    pub fn from_model_parameters(params: &BTreeMap<String, Value>) -> Self {
        let max_tokens = params
            .get("max_tokens")
            .and_then(Value::as_i64)
            .or_else(|| params.get("max_completion_tokens").and_then(Value::as_i64));
        Self {
            temperature: params.get("temperature").and_then(Value::as_f64),
            top_p: params.get("top_p").and_then(Value::as_f64),
            top_k: params.get("top_k").and_then(Value::as_i64),
            max_tokens,
            frequency_penalty: params.get("frequency_penalty").and_then(Value::as_f64),
            presence_penalty: params.get("presence_penalty").and_then(Value::as_f64),
            stop_sequences: params.get("stop").and_then(|value| {
                value
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .or_else(|| value.as_str().map(|item| vec![item.to_owned()]))
            }),
            seed: params.get("seed").and_then(Value::as_i64),
        }
    }
}

#[path = "request_params/tests.rs"]
mod tests;
