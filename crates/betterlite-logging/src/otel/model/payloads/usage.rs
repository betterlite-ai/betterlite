use super::super::utils::coercion::{OtelModelValue, as_int};
use super::helpers::cache_token_value;
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
    #[must_use]
    pub fn uncached_input_tokens(&self) -> Option<i64> {
        self.input_tokens.map(|input| {
            input
                .saturating_sub(self.cache_read_input_tokens.unwrap_or(0))
                .saturating_sub(self.cache_creation_input_tokens.unwrap_or(0))
                .max(0)
        })
    }
    #[must_use]
    pub fn non_reasoning_output_tokens(&self) -> Option<i64> {
        self.output_tokens.map(|output| {
            match output.checked_sub(self.reasoning_tokens.unwrap_or(0)) {
                Some(remaining) if remaining > 0 => remaining,
                Some(_) | None => 0,
            }
        })
    }
    #[must_use]
    pub fn from_payload(payload: &BTreeMap<String, OtelModelValue>) -> Self {
        let details = usage_details(payload);
        Self {
            input_tokens: payload.get("prompt_tokens").and_then(as_int),
            output_tokens: payload.get("completion_tokens").and_then(as_int),
            total_tokens: payload.get("total_tokens").and_then(as_int),
            cache_creation_input_tokens: cache_tokens(&[
                details
                    .usage_object
                    .and_then(|fields| fields.get("cache_creation_input_tokens")),
                details
                    .prompt_details
                    .and_then(|fields| fields.get("cache_write_tokens")),
                details
                    .prompt_details
                    .and_then(|fields| fields.get("cache_creation_tokens")),
                details
                    .prompt_details
                    .and_then(|fields| fields.get("cache_creation_input_tokens")),
            ]),
            cache_read_input_tokens: cache_tokens(&[
                details
                    .usage_object
                    .and_then(|fields| fields.get("cache_read_input_tokens")),
                details
                    .prompt_details
                    .and_then(|fields| fields.get("cached_tokens")),
                details
                    .usage_object
                    .and_then(|fields| fields.get("prompt_cache_hit_tokens")),
            ]),
            reasoning_tokens: cache_tokens(&[details
                .completion_details
                .and_then(|fields| fields.get("reasoning_tokens"))]),
        }
    }
}

struct UsageDetails<'usage> {
    usage_object: Option<&'usage BTreeMap<String, OtelModelValue>>,
    prompt_details: Option<&'usage BTreeMap<String, OtelModelValue>>,
    completion_details: Option<&'usage BTreeMap<String, OtelModelValue>>,
}

fn usage_details(payload: &BTreeMap<String, OtelModelValue>) -> UsageDetails<'_> {
    let usage_object = payload
        .get("metadata")
        .and_then(OtelModelValue::as_object)
        .and_then(|metadata| metadata.get("usage_object"))
        .and_then(OtelModelValue::as_object);
    let prompt_details = usage_object
        .and_then(|usage_fields| usage_fields.get("prompt_tokens_details"))
        .and_then(OtelModelValue::as_object);
    let completion_details = usage_object
        .and_then(|usage_fields| usage_fields.get("completion_tokens_details"))
        .and_then(OtelModelValue::as_object);
    UsageDetails {
        usage_object,
        prompt_details,
        completion_details,
    }
}

fn cache_tokens(raw_values: &[Option<&OtelModelValue>]) -> Option<i64> {
    let values = raw_values
        .iter()
        .filter_map(|value| (*value).cloned())
        .collect::<Vec<_>>();
    cache_token_value(&values)
}

#[path = "usage/tests.rs"]
mod tests;
