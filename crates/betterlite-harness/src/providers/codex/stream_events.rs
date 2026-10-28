use crate::events::Event;

use super::stream::{self, CodexStreamState, FailureDetail, Item, ItemEventKind};

pub(super) fn transform_item_event(
    item: Item,
    event_type: ItemEventKind,
    state: &mut CodexStreamState,
) -> Vec<Event> {
    match item {
        Item::AgentMessage { text, .. } => {
            if event_type != ItemEventKind::Completed {
                return Vec::new();
            }
            let message = text.unwrap_or_default();
            state.final_text.clone_from(&message);
            if message.is_empty() {
                Vec::new()
            } else {
                vec![Event::Text { delta: message }]
            }
        }
        Item::Reasoning { text, .. } => {
            if event_type != ItemEventKind::Completed {
                return Vec::new();
            }
            let Some(reasoning) = text.filter(|value| !value.is_empty()) else {
                return Vec::new();
            };
            vec![Event::Reasoning { delta: reasoning }]
        }
        Item::CommandExecution { .. }
        | Item::FileChange { .. }
        | Item::WebSearch { .. }
        | Item::McpToolCall { .. } => tool_events(&item, event_type, state),
        Item::Other => Vec::new(),
    }
}

pub(super) fn record_thread_id(thread_id: Option<String>, state: &mut CodexStreamState) {
    if let Some(reported) = thread_id.filter(|value| !value.is_empty()) {
        state.thread_id = Some(reported);
    }
}

pub(super) fn record_error(message: Option<String>, state: &mut CodexStreamState) {
    state.error = Some(
        message
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "codex reported an error".to_owned()),
    );
}

pub(super) fn record_turn_failed(error: Option<FailureDetail>, state: &mut CodexStreamState) {
    state.failed = true;
    let message = match error {
        Some(FailureDetail::Text(text)) => Some(text),
        Some(FailureDetail::Object { message }) => message,
        None => None,
    };
    state.error = message
        .filter(|text| !text.is_empty())
        .or_else(|| state.error.clone())
        .or_else(|| Some("codex turn failed".to_owned()));
}

fn tool_events(item: &Item, event_type: ItemEventKind, state: &mut CodexStreamState) -> Vec<Event> {
    let id = stream::item_id(item).to_owned();
    let mut events = Vec::new();
    if state.started.insert(id.clone()) {
        let (name, native_name, input, builtin) = stream::tool_input(item);
        events.push(Event::ToolCall {
            id: id.clone(),
            name,
            native_name,
            input,
            builtin,
        });
    }
    if event_type == ItemEventKind::Completed {
        let (output, is_error) = stream::tool_output(item);
        events.push(Event::ToolResult {
            id,
            output,
            is_error,
        });
    }
    events
}
