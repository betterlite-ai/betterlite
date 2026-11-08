use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;

use crate::selection::uses_legacy_message_accounting;
use crate::tools::{ToolLimits, format_function_definitions, parse_tool_definitions};

use super::error::CountError;
use super::request::{ContentPart, Message, MessageContent, Part, ToolChoice};
use super::text::Texter;

const REPLY_PRIMING_TOKENS: usize = 3;
const TOOL_DEFINITION_TOKENS: usize = 9;
const SYSTEM_TOOL_DISCOUNT: usize = 4;
const NAMED_CHOICE_TOKENS: usize = 7;
const NO_CHOICE_TOKENS: usize = 1;

#[derive(Debug, Clone, Copy)]
pub struct MessageParams {
    pub tokens_per_message: usize,
    pub tokens_per_name: isize,
}

#[must_use]
pub fn message_params(model: &str, catalog: &ModelCatalog) -> MessageParams {
    if uses_legacy_message_accounting(model, catalog) {
        return MessageParams {
            tokens_per_message: 4,
            tokens_per_name: -1,
        };
    }
    MessageParams {
        tokens_per_message: 3,
        tokens_per_name: 1,
    }
}

pub async fn count_messages(
    texter: &Texter<'_>,
    params: MessageParams,
    messages: &[Message],
) -> Result<usize, CountError> {
    let mut total = 0_usize;
    for message in messages {
        let count = count_message(texter, params, message).await?;
        total = total.checked_add(count).ok_or(CountError::Overflow)?;
    }
    Ok(total)
}

async fn count_message(
    texter: &Texter<'_>,
    params: MessageParams,
    message: &Message,
) -> Result<usize, CountError> {
    let mut total = params.tokens_per_message;
    for text in text_fields(message) {
        total = total
            .checked_add(texter.count(text).await?)
            .ok_or(CountError::Overflow)?;
    }
    if let Some(content) = &message.content {
        total = total
            .checked_add(count_content(texter, content).await?)
            .ok_or(CountError::Overflow)?;
    }
    if let Some(results) = &message.search_results {
        total = total
            .checked_add(count_search_results(texter, results).await?)
            .ok_or(CountError::Overflow)?;
    }
    if message.name.is_some() {
        total = total
            .checked_add_signed(params.tokens_per_name)
            .ok_or(CountError::Overflow)?;
    }
    Ok(total)
}

// every string field LiteLLM counts through its isinstance(str) branch, in any order because the sums commute
fn text_fields(message: &Message) -> Vec<&str> {
    let mut texts: Vec<&str> = vec![&message.role];
    if let Some(name) = &message.name {
        texts.push(name);
    }
    if let Some(id) = &message.tool_call_id {
        texts.push(id);
    }
    if let Some(reasoning) = &message.reasoning_content {
        texts.push(reasoning);
    }
    if let Some(call) = &message.function_call {
        texts.push(&call.arguments);
    }
    if let Some(calls) = &message.tool_calls {
        for call in calls {
            texts.push(&call.function.arguments);
        }
    }
    for value in message.passthrough.values() {
        if let Json::Text(text) = value {
            texts.push(text);
        }
    }
    texts
}

async fn count_content(texter: &Texter<'_>, content: &MessageContent) -> Result<usize, CountError> {
    let parts = match content {
        MessageContent::Text(text) => return texter.count(text).await,
        MessageContent::Parts(parts) => parts,
    };
    let mut total = 0_usize;
    for part in parts {
        let text = match part {
            Part::Text(text) => text.as_str(),
            Part::Block(block) if block.kind == "text" => block_text(block)?,
            Part::Block(block) => {
                return Err(CountError::UnsupportedContent {
                    kind: block.kind.clone(),
                });
            }
        };
        total = total
            .checked_add(texter.count(text).await?)
            .ok_or(CountError::Overflow)?;
    }
    Ok(total)
}

fn block_text(block: &ContentPart) -> Result<&str, CountError> {
    match block.fields.get("text") {
        None => Ok(""),
        Some(Json::Text(text)) => Ok(text.as_str()),
        Some(_) => Err(CountError::ContentField {
            kind: "text",
            field: "text",
        }),
    }
}

pub async fn count_extra(
    texter: &Texter<'_>,
    tools: Option<&[Json]>,
    tool_choice: Option<&ToolChoice>,
    includes_system_message: bool,
) -> Result<usize, CountError> {
    let mut total = REPLY_PRIMING_TOKENS;
    if let Some(definitions) = tools.filter(|list| !list.is_empty()) {
        total = total
            .checked_add(count_tool_definitions(texter, definitions).await?)
            .ok_or(CountError::Overflow)?;
        total = total
            .checked_add(TOOL_DEFINITION_TOKENS)
            .ok_or(CountError::Overflow)?;
        if includes_system_message {
            total = total
                .checked_sub(SYSTEM_TOOL_DISCOUNT)
                .ok_or(CountError::Overflow)?;
        }
    }
    match tool_choice {
        Some(ToolChoice::Mode(mode)) if mode == "none" => {
            total = total
                .checked_add(NO_CHOICE_TOKENS)
                .ok_or(CountError::Overflow)?;
        }
        Some(ToolChoice::Named(named)) => {
            total = total
                .checked_add(NAMED_CHOICE_TOKENS)
                .ok_or(CountError::Overflow)?;
            total = total
                .checked_add(texter.count(&named.function.name).await?)
                .ok_or(CountError::Overflow)?;
        }
        Some(ToolChoice::Mode(_)) | None => {}
    }
    Ok(total)
}

async fn count_tool_definitions(
    texter: &Texter<'_>,
    definitions: &[Json],
) -> Result<usize, CountError> {
    let bytes = serde_json::to_vec(definitions).map_err(|source| CountError::Json { source })?;
    let parsed = parse_tool_definitions(&bytes, ToolLimits::default())?;
    let formatted = format_function_definitions(&parsed)?;
    texter.count(&formatted).await
}

// LiteLLM's string branch runs before the search_results branch, so a string value is counted as it stands
async fn count_search_results(texter: &Texter<'_>, results: &Json) -> Result<usize, CountError> {
    if let Json::Text(text) = results {
        return texter.count(text).await;
    }
    let text = search_results_text(results)?;
    if text.is_empty() {
        return Ok(0);
    }
    texter.count(&text).await
}

fn search_results_text(results: &Json) -> Result<String, CountError> {
    let Json::List(entries) = results else {
        return Ok(String::new());
    };
    let mut text = String::new();
    for entry in entries {
        text.push_str(&search_result_text(entry)?);
    }
    Ok(text)
}

fn search_result_text(entry: &Json) -> Result<String, CountError> {
    let Json::Object(fields) = entry else {
        return Ok(String::new());
    };
    let mut text = String::new();
    for key in ["source", "title"] {
        if let Some(Json::Text(value)) = fields.get(key) {
            text.push_str(value);
        }
    }
    text.push_str(&content_blocks_text(fields.get("content")));
    if let Some(citations) = fields.get("citations") {
        let compact =
            serde_json::to_string(citations).map_err(|source| CountError::Json { source })?;
        text.push_str(&compact);
    }
    Ok(text)
}

fn content_blocks_text(content: Option<&Json>) -> String {
    let Some(Json::List(blocks)) = content else {
        return String::new();
    };
    let mut text = String::new();
    for block in blocks {
        if let Json::Object(fields) = block
            && let Some(Json::Text(value)) = fields.get("text")
        {
            text.push_str(value);
        }
    }
    text
}
