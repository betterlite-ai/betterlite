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

// Every Unicode decimal-digit block through 15.1, the digit set Python int() accepts; each block runs consecutively and the mathematical block holds five decades.
const DECIMAL_DIGITS: [(u32, u32); 64] = [
    (0x0030, 0x0039),
    (0x0660, 0x0669),
    (0x06F0, 0x06F9),
    (0x07C0, 0x07C9),
    (0x0966, 0x096F),
    (0x09E6, 0x09EF),
    (0x0A66, 0x0A6F),
    (0x0AE6, 0x0AEF),
    (0x0B66, 0x0B6F),
    (0x0BE6, 0x0BEF),
    (0x0C66, 0x0C6F),
    (0x0CE6, 0x0CEF),
    (0x0D66, 0x0D6F),
    (0x0DE6, 0x0DEF),
    (0x0E50, 0x0E59),
    (0x0ED0, 0x0ED9),
    (0x0F20, 0x0F29),
    (0x1040, 0x1049),
    (0x1090, 0x1099),
    (0x17E0, 0x17E9),
    (0x1810, 0x1819),
    (0x1946, 0x194F),
    (0x19D0, 0x19D9),
    (0x1A80, 0x1A89),
    (0x1A90, 0x1A99),
    (0x1B50, 0x1B59),
    (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49),
    (0x1C50, 0x1C59),
    (0xA620, 0xA629),
    (0xA8D0, 0xA8D9),
    (0xA900, 0xA909),
    (0xA9D0, 0xA9D9),
    (0xA9F0, 0xA9F9),
    (0xAA50, 0xAA59),
    (0xABF0, 0xABF9),
    (0xFF10, 0xFF19),
    (0x104A0, 0x104A9),
    (0x10D30, 0x10D39),
    (0x11066, 0x1106F),
    (0x110F0, 0x110F9),
    (0x11136, 0x1113F),
    (0x111D0, 0x111D9),
    (0x112F0, 0x112F9),
    (0x11450, 0x11459),
    (0x114D0, 0x114D9),
    (0x11650, 0x11659),
    (0x116C0, 0x116C9),
    (0x11730, 0x11739),
    (0x118E0, 0x118E9),
    (0x11950, 0x11959),
    (0x11C50, 0x11C59),
    (0x11D50, 0x11D59),
    (0x11DA0, 0x11DA9),
    (0x11F50, 0x11F59),
    (0x16A60, 0x16A69),
    (0x16AC0, 0x16AC9),
    (0x16B50, 0x16B59),
    (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149),
    (0x1E2F0, 0x1E2F9),
    (0x1E4F0, 0x1E4F9),
    (0x1E950, 0x1E959),
    (0x1FBF0, 0x1FBF9),
];

fn decimal_digit(character: char) -> Option<u32> {
    let code = u32::from(character);
    let (first, _) = DECIMAL_DIGITS
        .iter()
        .find(|(first, last)| (*first..=*last).contains(&code))?;
    code.checked_sub(*first)?.checked_rem(10)
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
            Some(TargetIndex::OutOfRange) => {
                warn!(
                    length = messages.len(),
                    "cache control injection index is out of bounds; skipping"
                );
                return Vec::new();
            }
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
        let digit = i128::from(decimal_digit(character)?);
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

const fn role_name(role: InjectionRole) -> &'static str {
    match role {
        InjectionRole::User => "user",
        InjectionRole::System => "system",
        InjectionRole::Assistant => "assistant",
    }
}
