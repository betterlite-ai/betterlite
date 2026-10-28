use crate::bedrock_records::{MessageFilterResult, MessageSelection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::slice;
use wire::{Content, Message, Part, Role};

pub mod content;
pub mod reconcile;
mod tests;
pub mod wire;

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scan {
    #[default]
    Request,
    Response,
}

#[derive(Debug, Default, Deserialize)]
pub struct SelectionContext {
    #[serde(default)]
    pub latest_only: bool,
    #[serde(default)]
    pub scan: Scan,
    pub structured_messages: Option<Vec<Message>>,
    pub messages: Option<Vec<Message>>,
    #[serde(default)]
    pub tool_results_only: bool,
}

#[must_use]
pub fn prepare_for_role(
    messages: Option<&[Message]>,
    latest_only: bool,
) -> MessageFilterResult<Message> {
    match messages {
        None => MessageFilterResult {
            payload_messages: None,
            original_messages: None,
            target_indices: None,
        },
        Some(items) if !latest_only => MessageFilterResult {
            payload_messages: Some(items.to_vec()),
            original_messages: None,
            target_indices: None,
        },
        Some(items) => match items
            .iter()
            .rposition(|message| message.role == Some(Role::User))
            .and_then(|index| items.get(index).map(|message| (index, message)))
        {
            Some((index, message)) => MessageFilterResult {
                payload_messages: Some(vec![message.clone()]),
                original_messages: Some(items.to_vec()),
                target_indices: Some(vec![index]),
            },
            None => MessageFilterResult {
                payload_messages: None,
                original_messages: None,
                target_indices: None,
            },
        },
    }
}

#[must_use]
pub fn count_message_texts(message: &Message) -> usize {
    match &message.content {
        Some(Content::Text(_)) => 1,
        Some(Content::List(parts)) => parts
            .iter()
            .filter(|part| matches!(part, Part::Block(block) if block.text.is_some()))
            .count(),
        None => 0,
    }
}

#[must_use]
pub fn locate_slice(
    messages: &[Message],
    target_index: usize,
    texts: &[String],
) -> Option<(usize, usize)> {
    let total = messages.iter().try_fold(0_usize, |count, message| {
        count.checked_add(count_message_texts(message))
    })?;
    let target = count_message_texts(messages.get(target_index)?);
    if total != texts.len() || target == 0 {
        return None;
    }
    let offset = messages
        .iter()
        .take(target_index)
        .try_fold(0_usize, |count, message| {
            count.checked_add(count_message_texts(message))
        })?;
    Some((offset, target))
}

#[must_use]
pub fn select(texts: &[String], context: &SelectionContext) -> MessageSelection<Message> {
    let structured = context
        .structured_messages
        .as_deref()
        .filter(|items| !items.is_empty())
        .or(context.messages.as_deref())
        .filter(|items| !items.is_empty());
    if !context.latest_only || matches!(context.scan, Scan::Response) || structured.is_none() {
        return legacy_selection(texts, context.latest_only);
    }
    let messages = structured.unwrap_or_default();
    let target = messages
        .iter()
        .rposition(|message| message.role == Some(Role::User))
        .and_then(|index| messages.get(index).map(|message| (index, message)));
    match target {
        Some((index, message)) if count_message_texts(message) > 0 => MessageSelection {
            filtered_messages: Some(vec![message.clone()]),
            scanned_slice: locate_slice(messages, index, texts),
            scanned_role_subset: true,
            skip_scan: false,
        },
        Some(_) | None => {
            if target.is_none() && context.tool_results_only {
                tracing::warn!(
                    "latest-user selection and tool-results-only filtering leave nothing to scan"
                );
            }
            MessageSelection {
                filtered_messages: None,
                scanned_slice: None,
                scanned_role_subset: true,
                skip_scan: true,
            }
        }
    }
}

fn legacy_selection(texts: &[String], latest_only: bool) -> MessageSelection<Message> {
    let selected = if latest_only {
        texts.last().map(slice::from_ref).unwrap_or_default()
    } else {
        texts
    };
    let mock = selected
        .iter()
        .map(|text| Message {
            role: Some(Role::User),
            content: Some(Content::Text(text.clone())),
            extra: BTreeMap::new(),
        })
        .collect();
    MessageSelection {
        filtered_messages: Some(mock),
        scanned_slice: None,
        scanned_role_subset: false,
        skip_scan: false,
    }
}
