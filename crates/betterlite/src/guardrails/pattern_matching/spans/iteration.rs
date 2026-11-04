use std::ops::Range;

use fancy_regex::{CompileError, Error, Regex, RegexBuilder, RegexInput};

use super::PatternError;

pub(super) fn matches(regex: &Regex, text: &str) -> Result<Vec<Range<usize>>, PatternError> {
    let mut spans = Vec::new();
    let mut position = 0;
    let mut consuming = None;
    // Position strictly advances each pass, so no more than text.len() + 1 passes ever run.
    for _ in 0..=text.len() {
        let Some(found) = regex.find_from_pos(text, position)? else {
            break;
        };
        spans.push(found.start()..found.end());
        position = found.end();
        if found.start() != found.end() {
            continue;
        }
        // Python retries at the same position with empty matches forbidden before advancing.
        let retry = consuming.get_or_insert_with(|| {
            RegexBuilder::new(regex.as_str())
                .find_not_empty(true)
                .build()
        });
        let nonempty = match retry {
            Ok(compiled) => {
                compiled.find_input(RegexInput::new(text).from_pos(position).anchored(true))?
            }
            Err(Error::CompileError(cause))
                if matches!(cause.as_ref(), CompileError::PatternCanNeverMatch) =>
            {
                None
            }
            Err(error) => return Err(PatternError::Regex(error.clone())),
        };
        if let Some(value) = nonempty {
            spans.push(value.start()..value.end());
            position = value.end();
        } else if let Some(character) = text.get(position..).and_then(|tail| tail.chars().next()) {
            position = position.saturating_add(character.len_utf8());
        } else {
            break;
        }
    }
    Ok(spans)
}
