use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;

use super::blocks::{field_text, json_truthy};
use super::contracts::{Conversation, History};
use super::error::{CompactionError, ErrorKind};
use super::items::{json_item, tool_ids_of};

#[must_use]
pub fn tool_ids(
    messages: &[ChatMessage],
    model: &str,
    results: bool,
) -> Result<Vec<String>, CompactionError> {
    let items: Vec<Json> = messages
        .iter()
        .map(|message| message_json(message, model))
        .collect::<Result<Vec<Json>, CompactionError>>()?;
    Ok(tool_ids_of(&items, results))
}

fn partition(items: &[Json], model: &str) -> Result<History, CompactionError> {
    let instruction_count = items
        .iter()
        .map(json_item)
        .take_while(|item| matches!(item.role, Some("system" | "developer")))
        .count();
    let conversation = items.get(instruction_count..).unwrap_or(&[]);
    if conversation
        .iter()
        .map(json_item)
        .any(|item| matches!(item.role, Some("system" | "developer")))
    {
        return Err(CompactionError {
            kind: ErrorKind::Unavailable,
            model: model.to_owned(),
            reason: "Mid-conversation instructions cannot be compacted".to_owned(),
        });
    }
    let split = conversation
        .iter()
        .map(json_item)
        .rposition(|item| {
            item.role == Some("user")
                && !item
                    .blocks
                    .iter()
                    .any(|block| block.kind == Some("tool_result"))
        })
        .unwrap_or(0);
    let prefix = conversation.get(..split).unwrap_or(&[]);
    let calls = tool_ids_of(prefix, false);
    let results = tool_ids_of(prefix, true);
    let mut sorted_calls = calls.clone();
    sorted_calls.sort();
    let mut sorted_results = results.clone();
    sorted_results.sort();
    let duplicated = sorted_calls.windows(2).any(|pair| pair[0] == pair[1]);
    if prefix.is_empty()
        || calls.iter().any(String::is_empty)
        || results.iter().any(String::is_empty)
        || duplicated
        || sorted_calls != sorted_results
    {
        return Err(CompactionError {
            kind: ErrorKind::Unavailable,
            model: model.to_owned(),
            reason: "No closed older conversation is available without changing the latest request"
                .to_owned(),
        });
    }
    Ok(History {
        instruction_count,
        prefix_count: split,
        tail_count: conversation.len().saturating_sub(split),
        calls,
        results,
    })
}

fn item_kind(item: &Json) -> Option<&str> {
    match item {
        Json::Object(fields) => Some(field_text(fields, "type").unwrap_or("message")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn text_blocks_only(item: &Json) -> bool {
    let Json::Object(fields) = item else {
        return true;
    };
    let Some(Json::List(blocks)) = fields.get("content") else {
        return true;
    };
    blocks.iter().all(|block| {
        matches!(
            block,
            Json::Object(fields)
                if matches!(field_text(fields, "type"), Some("input_text" | "output_text" | "text"))
        )
    })
}

fn function_tools_only(tools: &[Json]) -> bool {
    tools.iter().all(|tool| {
        matches!(
            tool,
            Json::Object(fields) if field_text(fields, "type") == Some("function")
        )
    })
}

fn reasoning_present(item: &Json) -> bool {
    let Json::Object(fields) = item else {
        return false;
    };
    if fields.get("thinking_blocks").is_some_and(json_truthy) {
        return true;
    }
    let Some(Json::List(blocks)) = fields.get("content") else {
        return false;
    };
    blocks.iter().any(|block| {
        matches!(
            block,
            Json::Object(fields)
                if matches!(
                    field_text(fields, "type"),
                    Some("thinking" | "redacted_thinking" | "compaction")
                )
        )
    })
}

fn responses_partition(
    items: &[Json],
    tools: &[Json],
    model: &str,
) -> Result<History, CompactionError> {
    if items.iter().any(|item| {
        !matches!(
            item_kind(item),
            Some("message" | "function_call" | "function_call_output")
        )
    }) {
        return Err(CompactionError {
            kind: ErrorKind::Unavailable,
            model: model.to_owned(),
            reason: "Opaque or provider-managed Responses items require client-managed native compaction"
                .to_owned(),
        });
    }
    if !items.iter().all(text_blocks_only) || !function_tools_only(tools) {
        return Err(CompactionError {
            kind: ErrorKind::Unavailable,
            model: model.to_owned(),
            reason: "Only text history and ordinary function tools support portable Responses compaction"
                .to_owned(),
        });
    }
    if items.iter().any(reasoning_present) {
        return Err(CompactionError {
            kind: ErrorKind::Unavailable,
            model: model.to_owned(),
            reason:
                "Native reasoning or compaction blocks require client-managed native compaction"
                    .to_owned(),
        });
    }
    partition(items, model)
}

#[must_use]
pub fn portable_history(
    conversation: &Conversation<'_>,
    model: &str,
) -> Result<History, CompactionError> {
    match conversation {
        Conversation::Text(_) => Err(CompactionError {
            kind: ErrorKind::Unavailable,
            model: model.to_owned(),
            reason: "A single user input cannot be compacted without changing the latest request"
                .to_owned(),
        }),
        Conversation::Messages(messages) => {
            let items: Vec<Json> = messages
                .iter()
                .map(|message| message_json(message, model))
                .collect::<Result<Vec<Json>, CompactionError>>()?;
            partition(&items, model)
        }
        Conversation::Responses { items, tools } => responses_partition(items, tools, model),
    }
}
