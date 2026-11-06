use std::collections::BTreeSet;

use super::inputs::{Inputs, StructuredMessage, message_texts};
use super::request_data::{
    LoggingObject, RequestData, ResponsesContent, ResponsesInput, ResponsesInputItem,
};
use crate::panw_prisma_airs::audit::Direction;
use crate::panw_prisma_airs::construct::PanwPrismaAirs;

// None means scan every text; Some is the set of text indices that come from a scannable message.
#[must_use]
pub fn scannable_text_indices(
    guardrail: &PanwPrismaAirs,
    inputs: &Inputs,
    request_data: &RequestData,
    logging_obj: Option<&LoggingObject>,
    direction: Direction,
) -> Option<BTreeSet<usize>> {
    if direction == Direction::Response {
        return None;
    }
    let messages = inputs
        .structured_messages
        .as_deref()
        .filter(|messages| !messages.is_empty())?;
    if use_latest_user_only(guardrail, request_data, logging_obj)
        && let Some(latest) = latest_user_text_indices(&inputs.texts, messages, request_data)
    {
        return Some(latest);
    }
    role_filtered_text_indices(&inputs.texts, messages)
}

#[must_use]
pub fn use_latest_user_only(
    guardrail: &PanwPrismaAirs,
    request_data: &RequestData,
    logging_obj: Option<&LoggingObject>,
) -> bool {
    guardrail
        .settings
        .latest_role_message_only
        .unwrap_or_else(|| is_anthropic_request(request_data, logging_obj))
}

#[must_use]
pub fn is_anthropic_request(
    request_data: &RequestData,
    logging_obj: Option<&LoggingObject>,
) -> bool {
    if let Some(call_type) = logging_obj.and_then(|logging| logging.call_type.as_deref())
        && call_type == "anthropic_messages"
    {
        return true;
    }
    request_data
        .proxy_server_request
        .as_ref()
        .and_then(|request| request.url.as_deref())
        .is_some_and(is_messages_path)
}

fn is_messages_path(url: &str) -> bool {
    let path = url::Url::parse(url).map_or_else(
        |_| url.split(['?', '#']).next().unwrap_or_default().to_owned(),
        |parsed| parsed.path().to_owned(),
    );
    path.trim_end_matches('/').ends_with("/v1/messages")
}

fn text_source_message_indices(
    texts: &[String],
    messages: &[StructuredMessage],
) -> Option<Vec<usize>> {
    let runs: Vec<Vec<&str>> = messages.iter().map(message_texts).collect();
    let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let forward = walk(&runs, &text_refs);
    if forward.len() != texts.len() {
        return None;
    }
    let reversed_runs: Vec<Vec<&str>> = runs
        .iter()
        .rev()
        .map(|run| run.iter().rev().copied().collect())
        .collect();
    let reversed_texts: Vec<&str> = text_refs.iter().rev().copied().collect();
    let backward_positions = walk(&reversed_runs, &reversed_texts);
    let last = messages.len().saturating_sub(1);
    let backward: Vec<usize> = backward_positions
        .iter()
        .rev()
        .map(|position| last.saturating_sub(*position))
        .collect();
    (forward == backward).then_some(forward)
}

// A message's texts are consumed only at the running position, so messages with no texts of their own are skipped.
fn walk(ordered_runs: &[Vec<&str>], ordered_texts: &[&str]) -> Vec<usize> {
    let mut sources = Vec::new();
    for (position, run) in ordered_runs.iter().enumerate() {
        let start = sources.len();
        let end = start.saturating_add(run.len());
        if ordered_texts.get(start..end) == Some(run.as_slice()) {
            sources.resize(end, position);
        }
    }
    sources
}

fn reasoning_item_text_indices(
    texts: &[String],
    request_data: &RequestData,
) -> Option<BTreeSet<usize>> {
    let items = match request_data.input.as_ref() {
        None | Some(ResponsesInput::Text(_)) => return Some(BTreeSet::new()),
        Some(ResponsesInput::Items(items)) => items,
    };
    let offset = usize::from(
        request_data
            .instructions
            .as_deref()
            .is_some_and(|text| !text.is_empty()),
    );
    let counts: Vec<usize> = items.iter().map(item_text_count).collect();
    let total = counts
        .iter()
        .try_fold(offset, |sum, count| sum.checked_add(*count))?;
    if total != texts.len() {
        return None;
    }
    let mut indices = BTreeSet::new();
    let mut start = offset;
    for (item, count) in items.iter().zip(counts) {
        if item.kind.as_deref() == Some("reasoning") {
            indices.extend(start..start.saturating_add(count));
        }
        start = start.saturating_add(count);
    }
    Some(indices)
}

fn item_text_count(item: &ResponsesInputItem) -> usize {
    match item.content.as_ref() {
        None => 0,
        Some(ResponsesContent::Text(_)) => 1,
        Some(ResponsesContent::Parts(parts)) => {
            parts.iter().filter(|part| part.text.is_some()).count()
        }
    }
}

fn latest_user_text_indices(
    texts: &[String],
    messages: &[StructuredMessage],
    request_data: &RequestData,
) -> Option<BTreeSet<usize>> {
    let sources = text_source_message_indices(texts, messages)?;
    let reasoning = reasoning_item_text_indices(texts, request_data)?;
    let reasoning_messages: BTreeSet<usize> = reasoning
        .iter()
        .filter_map(|index| sources.get(*index).copied())
        .collect();
    let latest_human = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(index, message)| {
            !reasoning_messages.contains(index)
                && matches!(message.role.as_deref(), Some("user" | "developer"))
        })
        .map(|(index, _)| index)?;
    let latest_message = messages.get(latest_human)?;
    if !sources.contains(&latest_human) && !message_texts(latest_message).is_empty() {
        return None;
    }
    Some(
        sources
            .iter()
            .enumerate()
            .filter(|(_, source)| **source == latest_human)
            .map(|(index, _)| index)
            .collect(),
    )
}

fn role_filtered_text_indices(
    texts: &[String],
    messages: &[StructuredMessage],
) -> Option<BTreeSet<usize>> {
    let sources = text_source_message_indices(texts, messages)?;
    Some(
        sources
            .iter()
            .enumerate()
            .filter(|(_, source)| {
                messages.get(**source).is_some_and(|message| {
                    matches!(
                        message.role.as_deref(),
                        Some("user" | "system" | "developer")
                    )
                })
            })
            .map(|(index, _)| index)
            .collect(),
    )
}

mod tests;
