use betterlite_prompts::arguments::json::{Json, Object};

use crate::straiker_webhook::types::StraikerWebhookUsage;

#[must_use]
pub fn response_finish_reason(response: &Json) -> Option<String> {
    let fields = response.as_object()?;
    if let Some(reason) = terminal_reason(fields.get("finish_reason"), fields.get("stop_reason")) {
        return Some(reason);
    }
    let Some(Json::List(choices)) = fields.get("choices") else {
        return None;
    };
    choices.iter().find_map(|choice| {
        let candidate = choice.as_object()?;
        terminal_reason(candidate.get("finish_reason"), candidate.get("stop_reason"))
    })
}

#[must_use]
pub fn response_usage(response: &Json) -> Option<StraikerWebhookUsage> {
    let usage = response.as_object()?.get("usage")?.as_object()?;
    let input_tokens = token_count(usage, "prompt_tokens", "input_tokens");
    let output_tokens = token_count(usage, "completion_tokens", "output_tokens");
    (input_tokens.is_some() || output_tokens.is_some()).then_some(StraikerWebhookUsage {
        input_tokens,
        output_tokens,
    })
}

fn terminal_reason(finish: Option<&Json>, stop: Option<&Json>) -> Option<String> {
    let reason = |value: Option<&Json>| {
        value
            .and_then(Json::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    reason(finish).or_else(|| reason(stop))
}

fn token_count(usage: &Object, count_key: &str, fallback_key: &str) -> Option<i64> {
    let count = |key: &str| match usage.get(key) {
        Some(Json::Number(number)) => number.as_i64(),
        _ => None,
    };
    count(count_key).or_else(|| count(fallback_key))
}

mod tests;
