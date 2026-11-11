use tracing::warn;

use crate::cache_accounting::points::{InjectionIndex, InjectionRole, MessageInjectionPoint};
use crate::message_content::Message;

#[derive(Clone, Copy)]
enum TargetIndex {
    Position(i64),
    OutOfRange,
}

enum Digits {
    Value(i128),
    Overflow,
}

#[must_use]
pub fn resolve_target_indices(point: &MessageInjectionPoint, messages: &[Message]) -> Vec<usize> {
    if let Some(index) = &point.index {
        let parsed = match index {
            InjectionIndex::Number(number) => Some(TargetIndex::Position(*number)),
            InjectionIndex::Text(text) => text_index(text),
        };
        match parsed {
            Some(TargetIndex::Position(position)) => {
                return resolve_index(position, messages.len());
            }
            Some(TargetIndex::OutOfRange) => return resolve_out_of_range(messages.len()),
            None => {}
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

// Python's int() takes surrounding whitespace, a sign and single underscores between digits; a value it accepts is an index even when the list cannot hold it.
fn text_index(text: &str) -> Option<TargetIndex> {
    let trimmed = text.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    let value = match digit_value(digits)? {
        Digits::Overflow => return Some(TargetIndex::OutOfRange),
        Digits::Value(value) => value,
    };
    let signed = if negative {
        let Some(signed) = value.checked_neg() else {
            return Some(TargetIndex::OutOfRange);
        };
        signed
    } else {
        value
    };
    i64::try_from(signed).map_or(Some(TargetIndex::OutOfRange), |index| {
        Some(TargetIndex::Position(index))
    })
}

fn digit_value(digits: &str) -> Option<Digits> {
    let mut value: i128 = 0;
    let mut seen_digit = false;
    let mut previous_underscore = false;
    for character in digits.chars() {
        if character == '_' {
            if !seen_digit || previous_underscore {
                return None;
            }
            previous_underscore = true;
            continue;
        }
        let digit = i128::from(character.to_digit(10)?);
        seen_digit = true;
        previous_underscore = false;
        let Some(multiplied) = value.checked_mul(10) else {
            return Some(Digits::Overflow);
        };
        let Some(next) = multiplied.checked_add(digit) else {
            return Some(Digits::Overflow);
        };
        value = next;
    }
    if !seen_digit || previous_underscore {
        return None;
    }
    Some(Digits::Value(value))
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

fn resolve_out_of_range(length: usize) -> Vec<usize> {
    warn!(
        length,
        "cache control injection index is out of bounds; skipping"
    );
    Vec::new()
}

const fn role_name(role: InjectionRole) -> &'static str {
    match role {
        InjectionRole::User => "user",
        InjectionRole::System => "system",
        InjectionRole::Assistant => "assistant",
    }
}
