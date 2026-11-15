use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;

use super::blocks::{counts_as_present, field_text};
use super::contracts::{Conversation, History};
use super::error::{CompactionError, ErrorKind};
use super::items::{json_item, tool_ids_of};

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

fn message_json(message: &ChatMessage, model: &str) -> Result<Json, CompactionError> {
    serde_json::to_value(message)
        .and_then(serde_json::from_value)
        .map_err(|source| CompactionError {
            kind: ErrorKind::Rejected,
            model: model.to_owned(),
            reason: "the chat message cannot be read as history".to_owned(),
            source: Some(source),
        })
}

fn unavailable(model: &str, reason: &str) -> CompactionError {
    CompactionError {
        kind: ErrorKind::Unavailable,
        model: model.to_owned(),
        reason: reason.to_owned(),
        source: None,
    }
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
        return Err(unavailable(
            model,
            "Mid-conversation instructions cannot be compacted",
        ));
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
    let (calls, results) = closed_ids(prefix, model)?;
    Ok(History {
        instruction_count,
        prefix_count: split,
        tail_count: conversation.len().saturating_sub(split),
        calls,
        results,
    })
}

fn closed_ids(prefix: &[Json], model: &str) -> Result<(Vec<String>, Vec<String>), CompactionError> {
    let calls = tool_ids_of(prefix, false);
    let results = tool_ids_of(prefix, true);
    let mut sorted_calls = calls.clone();
    sorted_calls.sort();
    let mut sorted_results = results.clone();
    sorted_results.sort();
    let duplicated = calls.iter().collect::<BTreeSet<_>>().len() != calls.len();
    let empty_id = calls.iter().chain(&results).any(String::is_empty);
    if prefix.is_empty() || empty_id || duplicated || sorted_calls != sorted_results {
        return Err(unavailable(
            model,
            "No closed older conversation is available without changing the latest request",
        ));
    }
    Ok((calls, results))
}

fn item_kind(item: &Json) -> Option<&str> {
    let Json::Object(fields) = item else {
        return None;
    };
    match fields.get("type") {
        None => Some("message"),
        Some(Json::Text(kind)) => Some(kind.as_str()),
        Some(_) => None,
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
            Json::Object(block_fields)
                if matches!(field_text(block_fields, "type"), Some("input_text" | "output_text" | "text"))
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
    if fields.get("thinking_blocks").is_some_and(counts_as_present) {
        return true;
    }
    let Some(Json::List(blocks)) = fields.get("content") else {
        return false;
    };
    blocks.iter().any(|block| {
        matches!(
            block,
            Json::Object(block_fields)
                if matches!(
                    field_text(block_fields, "type"),
                    Some("thinking" | "redacted_thinking" | "compaction")
                )
        )
    })
}

fn responses_checks(items: &[Json], tools: &[Json], model: &str) -> Result<(), CompactionError> {
    if items.iter().any(|item| {
        !matches!(
            item_kind(item),
            Some("message" | "function_call" | "function_call_output")
        )
    }) {
        return Err(unavailable(
            model,
            "Opaque or provider-managed Responses items require client-managed native compaction",
        ));
    }
    if !items.iter().all(text_blocks_only) || !function_tools_only(tools) {
        return Err(unavailable(
            model,
            "Only text history and ordinary function tools support portable Responses compaction",
        ));
    }
    Ok(())
}

pub fn portable_history(
    conversation: &Conversation<'_>,
    model: &str,
) -> Result<History, CompactionError> {
    let items: Vec<Json> = match conversation {
        Conversation::Text(_) => {
            return Err(unavailable(
                model,
                "A single user input cannot be compacted without changing the latest request",
            ));
        }
        Conversation::Messages(messages) => messages
            .iter()
            .map(|message| message_json(message, model))
            .collect::<Result<Vec<Json>, CompactionError>>()?,
        Conversation::Responses { items, tools } => {
            responses_checks(items, tools, model)?;
            items.to_vec()
        }
    };
    if items.iter().any(reasoning_present) {
        return Err(unavailable(
            model,
            "Native reasoning or compaction blocks require client-managed native compaction",
        ));
    }
    partition(&items, model)
}
