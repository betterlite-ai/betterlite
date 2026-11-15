use betterlite_core::api::chat_response_stream_tools::fields::string_value;
use betterlite_core::api::responses_tool_namespaces::values::has_value;
use betterlite_core::shadow_evidence::chat_text;
use betterlite_models::decisions::Json;

mod tests;

pub const MAX_JUDGE_RESPONSE_CHARS: usize = 8_000;
pub const MAX_JUDGE_PROMPT_CHARS: usize = 24_000;
pub const MAX_JUDGE_TOOL_DEFS_CHARS: usize = 2_000;
pub const JUDGE_MAX_OUTPUT_TOKENS: u64 = 4096;

pub const PAIRWISE_JUDGE_SYSTEM_PROMPT: &str = r#"You are an impartial quality judge comparing two responses to the same conversation.

The responses are labeled A and B in random order. You do not know which system produced which.

A response may be prose, or a tool call shown as `[tool call] name(arguments)` if the
assistant chose to act instead of answering directly. A tool call is not a defect: judge
whether calling that tool was the right response to the conversation, the same as you
would judge prose.

Criteria: correctness, completeness, clarity, conciseness.

Return ONLY valid JSON in this exact format, no other text:
{
  "preference": "A" | "B" | "tie",
  "confidence": <0.0 to 1.0>
}"#;

#[must_use]
pub fn tool_definitions_text(tools: &Json) -> String {
    let Json::List(entries) = tools else {
        return String::new();
    };
    let mut lines: Vec<String> = Vec::new();
    for entry in entries {
        if matches!(entry, Json::Text(_)) {
            continue;
        }
        lines.push(tool_line(entry));
    }
    if lines.is_empty() {
        return String::new();
    }
    let text = format!("Tools available to both responses:\n{}", lines.join("\n"));
    text.chars().take(MAX_JUDGE_TOOL_DEFS_CHARS).collect()
}

fn tool_line(entry: &Json) -> String {
    let function = chat_text::field(entry, "function").filter(|value| has_value(value));
    let custom = chat_text::field(entry, "custom").filter(|value| has_value(value));
    let payload = function.or(custom).unwrap_or(entry);
    let name = chat_text::field(payload, "name")
        .filter(|value| has_value(value))
        .map_or_else(|| "unnamed".to_owned(), string_value);
    let description = chat_text::field(payload, "description")
        .filter(|value| has_value(value))
        .map_or_else(|| "no description".to_owned(), string_value);
    format!("- {name}: {description}")
}

#[must_use]
pub fn judge_user_prompt(
    conversation: &str,
    response_a: &str,
    response_b: &str,
    tool_definitions: &str,
) -> String {
    let reply_a: String = response_a.chars().take(MAX_JUDGE_RESPONSE_CHARS).collect();
    let reply_b: String = response_b.chars().take(MAX_JUDGE_RESPONSE_CHARS).collect();
    let prefix = if tool_definitions.is_empty() {
        String::new()
    } else {
        format!("{tool_definitions}\n\n")
    };
    let budget = MAX_JUDGE_PROMPT_CHARS
        .saturating_sub(reply_a.chars().count())
        .saturating_sub(reply_b.chars().count())
        .saturating_sub(prefix.chars().count());
    let skip = conversation.chars().count().saturating_sub(budget);
    let tail: String = conversation.chars().skip(skip).collect();
    format!(
        "{prefix}Conversation:\n{tail}\n\nResponse A:\n{reply_a}\n\nResponse B:\n{reply_b}\n\nWhich response is better?"
    )
}
