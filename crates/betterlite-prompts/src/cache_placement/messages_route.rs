use crate::arguments::json::{Json, Object};
use crate::cache_accounting::census::{points_with_a_slot_left, reserved_outside_messages};
use crate::cache_accounting::constants::MAX_CACHE_CONTROL_BLOCKS;
use crate::cache_accounting::control::default_control;
use crate::cache_accounting::points::{
    CacheControlInjectionPoint, InjectionRole, MessageInjectionPoint,
};
use crate::cache_placement::marks::{count_request_marks, count_system_marks, system_has_mark};
use crate::cache_placement::message_injections::apply_message_injections;
use crate::cache_placement::points::message_point;
use crate::cache_placement::system::{SystemPrompt, marked_system};
use crate::message_content::{Content, ContentPart, Message};

pub struct MessagesInjection {
    pub messages: Vec<Message>,
    pub system: Option<SystemPrompt>,
    pub remaining: Vec<CacheControlInjectionPoint>,
}

struct SplitPoints<'input> {
    system: Vec<&'input MessageInjectionPoint>,
    placed: Vec<&'input MessageInjectionPoint>,
    remaining: Vec<CacheControlInjectionPoint>,
}

#[must_use]
pub fn apply_messages_route(
    messages: &[Message],
    system: Option<&SystemPrompt>,
    points: &[CacheControlInjectionPoint],
    openai_dialect: bool,
    external_breakpoints: usize,
) -> MessagesInjection {
    if points.is_empty() {
        return MessagesInjection {
            messages: messages.to_vec(),
            system: system.cloned(),
            remaining: Vec::new(),
        };
    }
    let SplitPoints {
        system: system_points,
        placed: placed_points,
        remaining,
    } = split_points(points);
    let reserved = reserved_outside_messages(&remaining, external_breakpoints, openai_dialect);
    let max_blocks = MAX_CACHE_CONTROL_BLOCKS.saturating_sub(reserved);
    let message_blocks = count_request_marks(messages, None);
    let (processed_system, system_blocks) = placed_system(
        system,
        &system_points,
        openai_dialect,
        max_blocks,
        message_blocks,
    );
    let converted: Vec<Message> = messages.iter().map(converted_message).collect();
    let budget = max_blocks.saturating_sub(system_blocks);
    let processed = apply_message_injections(&placed_points, &converted, budget, openai_dialect);
    let on_wire = count_request_marks(&processed, processed_system.as_ref())
        .saturating_add(external_breakpoints);
    let forwarded = points_with_a_slot_left(&remaining, on_wire, openai_dialect);
    MessagesInjection {
        messages: processed,
        system: processed_system,
        remaining: forwarded,
    }
}

fn split_points(points: &[CacheControlInjectionPoint]) -> SplitPoints<'_> {
    let message: Vec<&MessageInjectionPoint> = points.iter().filter_map(message_point).collect();
    let system = message
        .iter()
        .copied()
        .filter(|point| point.role == Some(InjectionRole::System))
        .collect();
    let placed = message
        .iter()
        .copied()
        .filter(|point| point.role != Some(InjectionRole::System))
        .collect();
    let remaining = points
        .iter()
        .filter(|point| message_point(point).is_none())
        .cloned()
        .collect();
    SplitPoints {
        system,
        placed,
        remaining,
    }
}

fn placed_system(
    system: Option<&SystemPrompt>,
    points: &[&MessageInjectionPoint],
    openai_dialect: bool,
    max_blocks: usize,
    message_blocks: usize,
) -> (Option<SystemPrompt>, usize) {
    let system_blocks = count_system_marks(system);
    let Some(prompt) = system else {
        return (None, system_blocks);
    };
    let Some(point) = points.first() else {
        return (Some(prompt.clone()), system_blocks);
    };
    if message_blocks.saturating_add(system_blocks) >= max_blocks || system_has_mark(prompt) {
        return (Some(prompt.clone()), system_blocks);
    }
    let control = point
        .control
        .clone()
        .unwrap_or_else(|| default_control(None));
    let (marked, added) = marked_system(prompt, &control, openai_dialect);
    let blocks = if added {
        system_blocks.saturating_add(1)
    } else {
        system_blocks
    };
    (Some(marked), blocks)
}

fn converted_message(message: &Message) -> Message {
    let Some(Content::Text(text)) = &message.content else {
        return message.clone();
    };
    let mut fields = Object::new();
    fields.insert("text".to_owned(), Json::Text(text.clone()));
    let block = ContentPart {
        kind: "text".to_owned(),
        fields,
    };
    let mut converted = message.clone();
    converted.content = Some(Content::Parts(vec![block]));
    converted
}
