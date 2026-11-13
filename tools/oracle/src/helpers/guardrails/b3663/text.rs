use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_text::{
    content_to_text, is_all_text_parts, merge_rewritten_text_parts,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentText {
    content: GuardrailValue,
    parts: Vec<GuardrailValue>,
    new_text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssistantText {
    pub(super) response: GuardrailValue,
}

pub(super) fn content_text(request: &ContentText) -> Value {
    json!({
        "text": content_to_text(&request.content),
        "all_text": is_all_text_parts(&request.content),
        "merged": merge_rewritten_text_parts(&request.parts, &request.new_text),
    })
}
