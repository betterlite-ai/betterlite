use std::fmt::Display;

use fancy_regex::{Captures, Error, Regex, RegexBuilder};

use error::PatternError;

use super::json_value::JsonValue;
use super::replacement_template::{expand, parse};

pub mod error;

const IGNORE_CASE: i64 = 2;
const MULTILINE: i64 = 8;
const DOT_ALL: i64 = 16;
const VERBOSE: i64 = 64;

// Python's ASCII flag (256) stays out because fancy-regex's ASCII mode rejects \W, \D, \S and "." on str input.
pub fn compile(pattern: &str, flags: i64) -> Result<Regex, Error> {
    RegexBuilder::new(pattern)
        .case_insensitive(flags & IGNORE_CASE != 0)
        .multi_line(flags & MULTILINE != 0)
        .dot_matches_new_line(flags & DOT_ALL != 0)
        .ignore_whitespace(flags & VERBOSE != 0)
        .build()
}

// A bad pattern never fails custom code: log it and answer with the fallback.
pub fn or_warn<Outcome, Failure>(
    outcome: Result<Outcome, Failure>,
    fallback: Outcome,
    primitive: &str,
) -> Outcome
where
    Failure: Display,
{
    match outcome {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "custom code {primitive} error");
            fallback
        }
    }
}

#[must_use]
pub fn regex_match_all(text: &str, pattern: &str, flags: i64) -> bool {
    let outcome = compile(pattern, flags)
        .and_then(|_| {
            compile(&format!("\\A(?:{pattern})\\z"), flags)
                .or_else(|_| compile(&format!("\\A(?:{pattern}\n)\\z"), flags))
        })
        .and_then(|regex| regex.is_match(text));
    or_warn(outcome, false, "regex_match_all")
}

// Python 3.7+ keeps an empty match where the previous match ended; fancy-regex's own iterator skips it.
fn matches_including_empty<'text>(
    regex: &Regex,
    text: &'text str,
) -> Result<Vec<Captures<'text, str>>, Error> {
    let mut found = Vec::new();
    let mut start = 0;
    while start <= text.len() {
        let Some(captured) = regex.captures_from_pos(text, start)? else {
            break;
        };
        let span = captured.get(0);
        let (begin, end) = span.map_or((start, start), |matched| (matched.start(), matched.end()));
        found.push(captured);
        if begin != end {
            start = end;
            continue;
        }
        if end == text.len() {
            break;
        }
        let step = text
            .get(end..)
            .and_then(|rest| rest.chars().next())
            .map_or(1, char::len_utf8);
        start = end.saturating_add(step);
    }
    Ok(found)
}

pub fn replace_all(
    text: &str,
    pattern: &str,
    replacement: &str,
    flags: i64,
) -> Result<String, PatternError> {
    let regex = compile(pattern, flags)?;
    let pieces = parse(replacement, &regex).ok_or(PatternError::Template)?;
    let mut replaced = String::new();
    let mut copied_to = 0;
    for captured in matches_including_empty(&regex, text)? {
        let Some(whole) = captured.get(0) else {
            continue;
        };
        replaced.push_str(text.get(copied_to..whole.start()).unwrap_or_default());
        replaced.push_str(&expand(&pieces, &captured));
        copied_to = whole.end();
    }
    replaced.push_str(text.get(copied_to..).unwrap_or_default());
    Ok(replaced)
}

pub fn find_all(text: &str, pattern: &str, flags: i64) -> Result<Vec<JsonValue>, Error> {
    let regex = compile(pattern, flags)?;
    let groups = regex.captures_len().saturating_sub(1);
    let mut found = Vec::new();
    for matched in matches_including_empty(&regex, text)? {
        let group_text = |index: usize| {
            matched
                .get(index)
                .map_or_else(String::new, |group| group.as_str().to_owned())
        };
        found.push(match groups {
            0 | 1 => JsonValue::String(group_text(groups)),
            _ => JsonValue::Array(
                (1..=groups)
                    .map(|index| JsonValue::String(group_text(index)))
                    .collect(),
            ),
        });
    }
    Ok(found)
}
