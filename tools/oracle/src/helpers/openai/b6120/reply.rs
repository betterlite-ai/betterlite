use serde::Deserialize;
use serde_json::{Map, Value, json};

use betterlite_openai::perplexity_chat::with_perplexity_fields;

use super::failure;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Steps {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Step {
    // the pinned Python call also names the model, id and logging start time
    body: String,
}

pub(super) fn run(steps: &Steps) -> HelperResult {
    let mut outcomes = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let reply: Value = serde_json::from_str(&step.body).map_err(failure)?;
        let enhanced = with_perplexity_fields(reply, step.body.as_bytes());
        outcomes.push(projection(&enhanced));
    }
    Ok(Value::Array(outcomes))
}

fn projection(reply: &Value) -> Value {
    let usage = reply.get("usage").map(usage_projection);
    let message = reply
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"));
    json!({
        "usage": usage,
        "annotations": message.and_then(|message| message.get("annotations")),
        "citations": reply.get("citations"),
        "search_results": reply.get("search_results"),
    })
}

fn usage_projection(usage: &Value) -> Value {
    let mut fields = Map::new();
    for name in [
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "citation_tokens",
    ] {
        let value = usage.get(name).cloned().unwrap_or(Value::Null);
        fields.insert(String::from(name), value);
    }
    let details = usage.get("prompt_tokens_details");
    let requests = details
        .and_then(|details| details.get("web_search_requests"))
        .cloned()
        .unwrap_or(Value::Null);
    fields.insert(String::from("web_search_requests"), requests);
    Value::Object(fields)
}
