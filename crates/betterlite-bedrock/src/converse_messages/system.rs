use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use betterlite_prompts::system_placement::text_parts::CONVERTED_SYSTEM_NOTE;

use crate::converse_messages::access::{assistant_has_tool_calls, opens_with_tool_result};
use crate::converse_messages::error::ConverseMessageError;
use crate::converse_params::cache_point::get_cache_point_block;
use crate::converse_params::wire::transcode;
use crate::model_routes::ParamMap;

const CACHE_CONTROL_FIELD: &str = "cache_control";
const TEXT_FIELD: &str = "text";
const TEXT_PART: &str = "text";

// The leading system run becomes Converse system blocks; the rest of the history stays in the messages.
pub fn transform_system_message(
    messages: &[Message],
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<(Vec<Message>, Vec<Json>), ConverseMessageError> {
    let leading_end = messages
        .iter()
        .position(|message| message.role != "system")
        .unwrap_or(messages.len());
    let mut system = Vec::new();
    for message in messages.iter().take(leading_end) {
        system.extend(system_blocks(message, model, model_cost)?);
    }
    let rest: Vec<Message> = messages.iter().skip(leading_end).cloned().collect();
    let mut reordered = Vec::with_capacity(rest.len());
    for index in 0..rest.len() {
        reordered.extend(reordered_around_tool_results(&rest, index));
    }
    let kept = reordered.iter().filter_map(converted_or_kept).collect();
    Ok((kept, system))
}

fn system_blocks(
    message: &Message,
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<Vec<Json>, ConverseMessageError> {
    let mut blocks = Vec::new();
    match message.content.as_ref() {
        Some(Content::Text(text)) if !text.is_empty() => {
            blocks.push(Json::Object(Object::from_iter([(
                TEXT_FIELD.to_owned(),
                Json::Text(text.clone()),
            )])));
            blocks.extend(cache_point(
                message.provider_specific_fields.get(CACHE_CONTROL_FIELD),
                model,
                model_cost,
            )?);
        }
        Some(Content::Parts(parts)) => {
            for part in parts {
                if part.kind == TEXT_PART
                    && let Some(text) = part
                        .fields
                        .get(TEXT_FIELD)
                        .filter(|value| value.is_truthy())
                {
                    blocks.push(Json::Object(Object::from_iter([(
                        TEXT_FIELD.to_owned(),
                        text.clone(),
                    )])));
                    blocks.extend(cache_point(
                        part.fields.get(CACHE_CONTROL_FIELD),
                        model,
                        model_cost,
                    )?);
                }
            }
        }
        Some(Content::Text(_)) | None => {}
    }
    Ok(blocks)
}

// B602 owns the cache point itself; a part carries its own control, a string message carries the message one.
fn cache_point(
    cache_control: Option<&Json>,
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<Option<Json>, ConverseMessageError> {
    let Some(control) = cache_control else {
        return Ok(None);
    };
    let block = ParamMap::from([(CACHE_CONTROL_FIELD.to_owned(), transcode(control)?)]);
    let Some(point) = get_cache_point_block(&block, model, model_cost)? else {
        return Ok(None);
    };
    Ok(Some(transcode(&point)?))
}

// A user turn between an assistant tool call and its results would split them, so the run moves after the results.
fn reordered_around_tool_results(messages: &[Message], index: usize) -> Vec<Message> {
    let Some(message) = messages.get(index) else {
        return Vec::new();
    };
    if opens_with_tool_result(message) {
        return tool_result_placement(messages, index, message);
    }
    if message.role != "system" {
        return vec![message.clone()];
    }
    let run_start = system_run_start(messages, index);
    let run_end = system_run_end(messages, index);
    let follower_opens = messages.get(run_end).is_some_and(opens_with_tool_result);
    let preceded_by_call = run_start
        .checked_sub(1)
        .and_then(|previous| messages.get(previous))
        .is_some_and(assistant_has_tool_calls);
    if follower_opens && preceded_by_call {
        return Vec::new();
    }
    vec![message.clone()]
}

fn tool_result_placement(messages: &[Message], index: usize, message: &Message) -> Vec<Message> {
    let next_opens = messages
        .get(index.saturating_add(1))
        .is_some_and(opens_with_tool_result);
    if next_opens {
        return vec![message.clone()];
    }
    let run_start = tool_run_start(messages, index);
    let run = system_run_before(messages, run_start);
    let previous = run_start.checked_sub(run.len().saturating_add(1));
    let moves_after_results = previous
        .filter(|_previous| !run.is_empty())
        .and_then(|previous| messages.get(previous))
        .is_some_and(assistant_has_tool_calls);
    if !moves_after_results {
        return vec![message.clone()];
    }
    let mut moved = Vec::with_capacity(run.len().saturating_add(1));
    moved.push(message.clone());
    moved.extend(run);
    moved
}

fn tool_run_start(messages: &[Message], index: usize) -> usize {
    messages
        .get(..index)
        .and_then(|before| {
            before
                .iter()
                .rposition(|message| !opens_with_tool_result(message))
        })
        .map_or(0, |last_other| last_other.saturating_add(1))
}

fn system_run_before(messages: &[Message], index: usize) -> Vec<Message> {
    let start = system_run_start(messages, index);
    messages
        .get(start..index)
        .map_or_else(Vec::new, |run| run.to_vec())
}

fn system_run_start(messages: &[Message], index: usize) -> usize {
    messages
        .get(..index)
        .and_then(|before| before.iter().rposition(|message| message.role != "system"))
        .map_or(0, |last_other| last_other.saturating_add(1))
}

fn system_run_end(messages: &[Message], index: usize) -> usize {
    messages
        .get(index..)
        .and_then(|after| after.iter().position(|message| message.role != "system"))
        .map_or(messages.len(), |offset| index.saturating_add(offset))
}

fn converted_or_kept(message: &Message) -> Option<Message> {
    if message.role != "system" {
        return Some(message.clone());
    }
    system_role_message_as_user(message)
}

fn system_role_message_as_user(message: &Message) -> Option<Message> {
    let blocks = converted_text_blocks(message);
    if blocks.is_empty() {
        return None;
    }
    let mut content = Vec::with_capacity(blocks.len().saturating_add(1));
    content.push(ContentPart {
        kind: TEXT_PART.to_owned(),
        fields: Object::from_iter([(
            TEXT_FIELD.to_owned(),
            Json::Text(CONVERTED_SYSTEM_NOTE.to_owned()),
        )]),
    });
    content.extend(blocks);
    Some(Message {
        role: "user".to_owned(),
        content: Some(Content::Parts(content)),
        name: None,
        provider_specific_fields: Object::new(),
    })
}

fn converted_text_blocks(message: &Message) -> Vec<ContentPart> {
    match message.content.as_ref() {
        Some(Content::Text(text)) if !text.is_empty() => vec![converted_text_block(
            Json::Text(text.clone()),
            message.provider_specific_fields.get(CACHE_CONTROL_FIELD),
        )],
        Some(Content::Parts(parts)) => parts
            .iter()
            .filter_map(|part| {
                if part.kind != TEXT_PART {
                    return None;
                }
                let text = part
                    .fields
                    .get(TEXT_FIELD)
                    .filter(|value| value.is_truthy())?;
                Some(converted_text_block(
                    text.clone(),
                    part.fields.get(CACHE_CONTROL_FIELD),
                ))
            })
            .collect(),
        Some(Content::Text(_)) | None => Vec::new(),
    }
}

// A converted system entry keeps its cache_control exactly as sent; only null counts as absent here.
fn converted_text_block(text: Json, cache_control: Option<&Json>) -> ContentPart {
    let mut fields = Object::from_iter([(TEXT_FIELD.to_owned(), text)]);
    if let Some(control) = cache_control.filter(|value| !matches!(value, Json::Null)) {
        fields.insert(CACHE_CONTROL_FIELD.to_owned(), control.clone());
    }
    ContentPart {
        kind: TEXT_PART.to_owned(),
        fields,
    }
}
