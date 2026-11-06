use super::REDACTED_BY_LITELM;

const QUOTED_KEYS: [&str; 3] = ["input", "messages", "prompt"];
const ASSIGN_KEY: &str = "input_value";

#[must_use]
pub fn redact_spend_error_text(text: &str) -> String {
    let redaction = format!("\"{REDACTED_BY_LITELM}\"");
    let mut output = String::new();
    let mut position = 0;
    loop {
        let Some(found) = next_key_start(text, position) else {
            if let Some(rest) = text.get(position..) {
                output.push_str(rest);
            }
            return output;
        };
        if let Some(prefix) = text.get(position..found.end) {
            output.push_str(prefix);
        }
        let Some(first) = text.get(found.end..).and_then(|rest| rest.chars().next()) else {
            return output;
        };
        if matches!(first, '[' | '{' | '\'' | '"') {
            output.push_str(&redaction);
            match scan_balanced_value_end(text, found.end) {
                Some(end) => position = end,
                None => return output,
            }
        } else {
            position = found.end;
        }
    }
}

struct KeyMatch {
    end: usize,
}

fn next_key_start(text: &str, from: usize) -> Option<KeyMatch> {
    let mut position = from;
    while position < text.len() {
        if let Some(end) = quoted_key_end(text, position) {
            return Some(KeyMatch { end });
        }
        if let Some(end) = assignment_end(text, position) {
            return Some(KeyMatch { end });
        }
        position = position.saturating_add(1);
    }
    None
}

fn quoted_key_end(text: &str, position: usize) -> Option<usize> {
    let quote = *text.as_bytes().get(position)?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let key_start = position.checked_add(1)?;
    let rest = text.get(key_start..)?;
    let key = QUOTED_KEYS.iter().find(|key| rest.starts_with(*key))?;
    let after_key = key_start.checked_add(key.len())?;
    let closing = *text.as_bytes().get(after_key)?;
    if closing != b'\'' && closing != b'"' {
        return None;
    }
    let separator = skip_spaces(text, after_key.checked_add(1)?);
    if text.as_bytes().get(separator) != Some(&b':') {
        return None;
    }
    Some(skip_spaces(text, separator.checked_add(1)?))
}

fn assignment_end(text: &str, position: usize) -> Option<usize> {
    let rest = text.get(position..)?;
    if !rest.starts_with(ASSIGN_KEY) {
        return None;
    }
    let previous = text
        .get(..position)
        .and_then(|head| head.chars().next_back());
    if previous.is_some_and(|character| character.is_alphanumeric() || character == '_') {
        return None;
    }
    let separator = skip_spaces(text, position.checked_add(ASSIGN_KEY.len())?);
    if text.as_bytes().get(separator) != Some(&b'=') {
        return None;
    }
    Some(skip_spaces(text, separator.checked_add(1)?))
}

// Python's \s also covers the four file separators Rust's is_whitespace leaves out
fn skip_spaces(text: &str, from: usize) -> usize {
    let mut position = from;
    while let Some(character) = text.get(position..).and_then(|rest| rest.chars().next()) {
        if !character.is_whitespace() && !matches!(character, '\u{1c}'..='\u{1f}') {
            break;
        }
        position = position.saturating_add(character.len_utf8());
    }
    position
}

fn scan_balanced_value_end(text: &str, start: usize) -> Option<usize> {
    let first = *text.as_bytes().get(start)?;
    if first == b'\'' || first == b'"' {
        return scan_quoted_string_end(text, start, first);
    }
    let close = match first {
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    };
    let mut depth = 0_u32;
    let mut position = start;
    while let Some(&byte) = text.as_bytes().get(position) {
        if byte == b'\'' || byte == b'"' {
            position = scan_quoted_string_end(text, position, byte)?;
            continue;
        }
        if byte == first {
            depth = depth.saturating_add(1);
        }
        if byte == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return position.checked_add(1);
            }
        }
        position = position.saturating_add(1);
    }
    None
}

fn scan_quoted_string_end(text: &str, start: usize, quote: u8) -> Option<usize> {
    let mut position = start.checked_add(1)?;
    while let Some(&byte) = text.as_bytes().get(position) {
        if byte == b'\\' {
            position = position.checked_add(2)?;
            continue;
        }
        if byte == quote {
            return position.checked_add(1);
        }
        position = position.saturating_add(1);
    }
    None
}
