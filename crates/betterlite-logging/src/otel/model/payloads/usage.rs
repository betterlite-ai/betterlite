use super::helpers::cache_token_value;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LLMUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

impl LLMUsage {
    pub fn uncached_input_tokens(&self) -> Option<i64> {
        self.input_tokens.map(|input| {
            (input
                - self.cache_read_input_tokens.unwrap_or(0)
                - self.cache_creation_input_tokens.unwrap_or(0))
            .max(0)
        })
    }
    pub fn non_reasoning_output_tokens(&self) -> Option<i64> {
        self.output_tokens
            .map(|output| (output - self.reasoning_tokens.unwrap_or(0)).max(0))
    }
    pub fn from_payload(payload: &BTreeMap<String, Value>) -> Self {
        let usage = payload
            .get("metadata")
            .and_then(|value| value.get("usage_object"))
            .and_then(Value::as_object);
        let prompt = usage
            .and_then(|value| value.get("prompt_tokens_details"))
            .and_then(Value::as_object);
        let completion = usage
            .and_then(|value| value.get("completion_tokens_details"))
            .and_then(Value::as_object);
        let source = |path: &[&str]| -> Vec<Value> {
            path.iter()
                .filter_map(|key| {
                    usage
                        .and_then(|map| map.get(*key))
                        .or_else(|| prompt.and_then(|map| map.get(*key)))
                        .cloned()
                })
                .collect()
        };
        Self {
            input_tokens: payload.get("prompt_tokens").and_then(Value::as_i64),
            output_tokens: payload.get("completion_tokens").and_then(Value::as_i64),
            total_tokens: payload.get("total_tokens").and_then(Value::as_i64),
            cache_creation_input_tokens: cache_token_value(&source(&[
                "cache_creation_input_tokens",
                "cache_write_tokens",
                "cache_creation_tokens",
            ])),
            cache_read_input_tokens: cache_token_value(&source(&[
                "cache_read_input_tokens",
                "cached_tokens",
                "prompt_cache_hit_tokens",
            ])),
            reasoning_tokens: cache_token_value(
                &completion
                    .and_then(|map| map.get("reasoning_tokens"))
                    .cloned()
                    .into_iter()
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

#[path = "usage/tests.rs"]
mod tests;
