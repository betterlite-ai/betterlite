use tracing::warn;

use crate::cache_accounting::points::{InjectionIndex, InjectionRole, MessageInjectionPoint};
use crate::message_content::Message;

#[must_use]
pub fn resolve_target_indices(point: &MessageInjectionPoint, messages: &[Message]) -> Vec<usize> {
    if let Some(index) = &point.index {
        let numeric = match index {
            InjectionIndex::Number(number) => Some(*number),
            InjectionIndex::Text(text) => text.trim().parse::<i64>().ok(),
        };
        if let Some(parsed) = numeric {
            return resolve_index(parsed, messages.len());
        }
    }
    let Some(target) = point.role else {
        return Vec::new();
    };
    let wanted = role_name(target);
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == wanted)
        .map(|(index, _)| index)
        .collect()
}

fn resolve_index(index: i64, length: usize) -> Vec<usize> {
    let Some(limit) = i64::try_from(length).ok() else {
        return Vec::new();
    };
    let normalized = if index < 0 {
        index.saturating_add(limit)
    } else {
        index
    };
    let candidate = usize::try_from(normalized)
        .ok()
        .filter(|position| *position < length);
    let Some(position) = candidate else {
        warn!(
            original = index,
            length, normalized, "cache control injection index is out of bounds; skipping"
        );
        return Vec::new();
    };
    vec![position]
}

const fn role_name(role: InjectionRole) -> &'static str {
    match role {
        InjectionRole::User => "user",
        InjectionRole::System => "system",
        InjectionRole::Assistant => "assistant",
    }
}
