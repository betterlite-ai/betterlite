use serde_json::Number;
use unicode_general_category::{GeneralCategory, get_general_category};

// int() and float() strip Python's whitespace set, which also holds the four separators Unicode's White_Space leaves out.
const PYTHON_SEPARATORS: [char; 4] = ['\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}'];

// int(text): Python accepts surrounding whitespace, a sign, single underscores between digits and any decimal digit script, at any width.
#[must_use]
pub fn scalar_int(text: &str) -> Option<Number> {
    let normalized = normalized_digits(text)?;
    let canonical = canonical_integer(&normalized)?;
    serde_json::from_str::<Number>(&canonical).ok()
}

// float(text): the same text rules, then Python's float grammar, which Rust's f64 parser accepts once the digits and underscores are normalized.
#[must_use]
pub fn scalar_float(text: &str) -> Option<f64> {
    let normalized = normalized_digits(text)?;
    let plain = without_underscores(&normalized)?;
    plain.parse::<f64>().ok()
}

fn normalized_digits(text: &str) -> Option<String> {
    let trimmed = text.trim_matches(|character: char| {
        character.is_whitespace() || PYTHON_SEPARATORS.contains(&character)
    });
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(trimmed.len());
    for character in trimmed.chars() {
        match decimal_value(character) {
            Some(value) => {
                let ascii = b'0'.checked_add(u8::try_from(value).ok()?)?;
                out.push(char::from(ascii));
            }
            None => out.push(character),
        }
    }
    Some(out)
}

// A decimal digit's value is its distance within the contiguous run of ten-digit blocks it sits in, so ١ is one and the first ١٢ pairs to 12.
fn decimal_value(character: char) -> Option<u32> {
    if get_general_category(character) != GeneralCategory::DecimalNumber {
        return None;
    }
    let mut value: u32 = 0;
    let mut current = character;
    while let Some(previous) = u32::from(current).checked_sub(1).and_then(char::from_u32) {
        if get_general_category(previous) != GeneralCategory::DecimalNumber {
            break;
        }
        value = value.saturating_add(1).checked_rem(10)?;
        current = previous;
    }
    Some(value)
}

// An underscore is only legal between two digits; everything else reaches the number parser unchanged.
fn without_underscores(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut previous_digit = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let next_digit = characters.peek().is_some_and(char::is_ascii_digit);
            if !previous_digit || !next_digit {
                return None;
            }
            previous_digit = false;
        } else {
            out.push(character);
            previous_digit = character.is_ascii_digit();
        }
    }
    Some(out)
}

fn canonical_integer(text: &str) -> Option<String> {
    let (negative, separated) = split_sign(text);
    let digits = without_underscores(separated)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let magnitude = digits.trim_start_matches('0');
    if magnitude.is_empty() {
        return Some("0".to_owned());
    }
    if negative {
        return Some(format!("-{magnitude}"));
    }
    Some(magnitude.to_owned())
}

fn split_sign(text: &str) -> (bool, &str) {
    if let Some(rest) = text.strip_prefix('-') {
        return (true, rest);
    }
    (false, text.strip_prefix('+').unwrap_or(text))
}
