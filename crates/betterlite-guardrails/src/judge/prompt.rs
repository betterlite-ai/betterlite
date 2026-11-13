use betterlite_config::sections::guardrail_judge::Criterion;
use betterlite_prompts::arguments::json::Json as ContentJson;
use betterlite_prompts::message_content::{Content, Message};
use betterlite_prompts::message_fields::last_user::get_last_user_message;

use crate::generic_guardrail_contracts::GuardrailInputType;

use super::apply::JudgeInputs;

pub const JUDGE_SYSTEM_PROMPT_REQUEST: &str = "You are a quality judge. Evaluate the request against the criteria provided.\nJudge the most recent user turn; treat earlier turns in the conversation only as context.\nFor each criterion, assign a score from 0 to 100 and provide concise reasoning.\nReturn ONLY valid JSON in this exact format:\n{\n  \"verdicts\": [\n    {\"criterion_name\": \"<name>\", \"score\": <0-100>, \"reasoning\": \"<one sentence>\", \"passed\": <true|false>, \"weight\": <weight>}\n  ],\n  \"overall_score\": <weighted average 0-100>\n}";

pub const JUDGE_SYSTEM_PROMPT_RESPONSE: &str = "You are a quality judge. Evaluate the assistant's response against the criteria provided.\nFor each criterion, assign a score from 0 to 100 and provide concise reasoning.\nReturn ONLY valid JSON in this exact format:\n{\n  \"verdicts\": [\n    {\"criterion_name\": \"<name>\", \"score\": <0-100>, \"reasoning\": \"<one sentence>\", \"passed\": <true|false>, \"weight\": <weight>}\n  ],\n  \"overall_score\": <weighted average 0-100>\n}";

#[must_use]
pub const fn system_prompt(input_type: GuardrailInputType) -> &'static str {
    match input_type {
        GuardrailInputType::Request => JUDGE_SYSTEM_PROMPT_REQUEST,
        GuardrailInputType::Response => JUDGE_SYSTEM_PROMPT_RESPONSE,
    }
}

#[must_use]
pub fn text_under_review(inputs: &JudgeInputs, input_type: GuardrailInputType) -> String {
    let all_text = inputs.texts.as_deref().unwrap_or_default().join("\n");
    if input_type == GuardrailInputType::Response {
        return all_text;
    }
    let messages = inputs.structured_messages.as_deref().unwrap_or_default();
    get_last_user_message(messages).unwrap_or(all_text)
}

// LiteLLM's _extract_text_from_content, applied to the typed message content the inputs carry.
fn content_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Parts(parts) => parts
            .iter()
            .filter(|part| part.kind == "text")
            .filter_map(|part| part.fields.get("text").and_then(ContentJson::as_str))
            .collect::<Vec<&str>>()
            .join(" "),
    }
}

fn conversation_block(messages: &[Message], input_type: GuardrailInputType) -> String {
    let conversation = messages
        .iter()
        .filter_map(|message| {
            let content = message.content.as_ref()?;
            Some(format!(
                "{}: {}",
                message.role.to_uppercase(),
                content_text(content)
            ))
        })
        .collect::<Vec<String>>()
        .join("\n");
    if conversation.is_empty() && input_type != GuardrailInputType::Response {
        return String::new();
    }
    format!("Conversation:\n{conversation}\n\n")
}

#[must_use]
pub fn build_judge_prompt(
    criteria: &[Criterion],
    messages: &[Message],
    text_under_review: &str,
    input_type: GuardrailInputType,
) -> String {
    let criteria_block = criteria
        .iter()
        .map(|criterion| {
            format!(
                "- {} (weight {}%): {}",
                criterion.name,
                criterion.weight.to_prompt_text(),
                criterion.description
            )
        })
        .collect::<Vec<String>>()
        .join("\n");
    let subject = match input_type {
        GuardrailInputType::Request => "Latest request turn to evaluate",
        GuardrailInputType::Response => "Assistant response to evaluate",
    };
    let conversation = conversation_block(messages, input_type);
    format!(
        "Criteria to evaluate:\n{criteria_block}\n\n{conversation}{subject}:\n{text_under_review}"
    )
}
