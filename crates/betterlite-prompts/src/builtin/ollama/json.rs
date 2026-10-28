use super::super::{PromptError, bounds};
use serde::Serialize;
use std::fmt::Write as _;
mod numbers;

pub(super) fn render<Value>(value: &Value, compact: bool) -> Result<String, PromptError>
where
    Value: Serialize,
{
    let pretty = serde_json::to_string_pretty(value)?;
    let normalized = pretty
        .lines()
        .map(numbers::line)
        .collect::<Vec<_>>()
        .join("\n");
    let mut output = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in normalized.chars() {
        if character == '"' && !escaped {
            quoted = !quoted;
        }
        escaped = character == '\\' && !escaped;
        if compact && !quoted && character.is_ascii_whitespace() {
            continue;
        }
        if character < '\u{7f}' {
            output.push(character);
        } else {
            for unit in character.encode_utf16(&mut [0; 2]) {
                write!(output, "\\u{unit:04x}")?;
            }
        }
    }
    bounds::finish(output)
}
