use std::fmt::Display;

use fancy_regex::{Error, Regex, RegexBuilder};

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
        .and_then(|_| compile(&format!("\\A(?:{pattern})\\z"), flags))
        .and_then(|regex| regex.is_match(text));
    or_warn(outcome, false, "regex_match_all")
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
    for captured in regex.captures_iter(text) {
        let found = captured?;
        let Some(whole) = found.get(0) else { continue };
        replaced.push_str(text.get(copied_to..whole.start()).unwrap_or_default());
        replaced.push_str(&expand(&pieces, &found));
        copied_to = whole.end();
    }
    replaced.push_str(text.get(copied_to..).unwrap_or_default());
    Ok(replaced)
}

pub fn find_all(text: &str, pattern: &str, flags: i64) -> Result<Vec<JsonValue>, Error> {
    let regex = compile(pattern, flags)?;
    let groups = regex.captures_len().saturating_sub(1);
    let mut found = Vec::new();
    for captured in regex.captures_iter(text) {
        let matched = captured?;
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
