use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::wire::{ChatMessage, Content, ContentItem, to_raw};

pub(super) struct TextTask {
    pub(super) message: usize,
    pub(super) part: Option<usize>,
    pub(super) text: String,
}

pub(super) fn request_messages(
    request: &RequestData,
) -> Result<Option<Vec<ChatMessage>>, PresidioResponseError> {
    let Some(raw) = request.get("messages") else {
        return Ok(None);
    };
    let text = raw.get().trim_start();
    if text == "null" {
        return Ok(None);
    }
    if !text.starts_with('[') {
        return Err(PresidioResponseError::MessagesNotList);
    }
    serde_json::from_str(text)
        .map(Some)
        .map_err(|source| PresidioResponseError::MessagesJson { source })
}

pub(super) fn collect_text_tasks(messages: &[ChatMessage]) -> Vec<TextTask> {
    let mut tasks = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        match message.content.as_ref() {
            Some(Content::Text(text)) => tasks.push(TextTask {
                message: message_index,
                part: None,
                text: text.clone(),
            }),
            Some(Content::Blocks(items)) => push_part_tasks(items, message_index, &mut tasks),
            Some(Content::Other(_)) | None => {}
        }
    }
    tasks
}

fn push_part_tasks(items: &[ContentItem], message: usize, tasks: &mut Vec<TextTask>) {
    for (part_index, item) in items.iter().enumerate() {
        let ContentItem::Object(part) = item else {
            continue;
        };
        let Some(text) = part.text.as_ref() else {
            continue;
        };
        tasks.push(TextTask {
            message,
            part: Some(part_index),
            text: text.clone(),
        });
    }
}

pub(super) fn apply_masked_texts(
    messages: &mut [ChatMessage],
    tasks: &[TextTask],
    masked: &[Option<String>],
) {
    for (task, masked_text) in tasks.iter().zip(masked) {
        if let Some(text) = masked_text.as_ref() {
            apply_text(messages, task, text);
        }
    }
}

fn apply_text(messages: &mut [ChatMessage], task: &TextTask, text: &str) {
    let Some(message) = messages.get_mut(task.message) else {
        return;
    };
    match task.part {
        None => message.content = Some(Content::Text(text.to_owned())),
        Some(index) => apply_part(message, index, text),
    }
}

fn apply_part(message: &mut ChatMessage, index: usize, text: &str) {
    let Some(Content::Blocks(items)) = message.content.as_mut() else {
        return;
    };
    let Some(ContentItem::Object(part)) = items.get_mut(index) else {
        return;
    };
    part.set_text(text.to_owned());
}

pub(super) fn store_messages(
    request: &mut RequestData,
    messages: &[ChatMessage],
) -> Result<(), PresidioResponseError> {
    request.insert("messages".to_owned(), to_raw(messages)?);
    Ok(())
}
