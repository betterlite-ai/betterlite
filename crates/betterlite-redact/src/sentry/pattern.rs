use std::ops::Range;

use regex::Regex;

use super::fixture::{self, GENERATED_KEY_LENGTH, KEY_PREFIX, MINIMUM_CUSTOM_KEY_LENGTH};

pub(super) const FILTERED: &str = "[Filtered]";

const QUOTED_VALUE: &str = r#"'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*""#;
const BRACKET_ATOM: &str = r#"(?:'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*")|[^\[\]{}()'"]"#;
const NESTED_BRACKET_LEVELS: usize = 3;
const BARE_VALUE: &str = r"[^,)\]}\s]+";
const EMAIL_BODY: &str = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}";
const SHA256_BODY: &str = "[0-9a-f]{64}";

#[derive(Debug, Clone, thiserror::Error)]
pub enum PatternError {
    #[error("sentry scrubbing pattern failed to compile: {reason}")]
    Compile { reason: String },
}

pub(super) struct Patterns {
    field: Regex,
    key: Regex,
    email: Option<Regex>,
    sha256: Option<Regex>,
}

fn bracketed_value_pattern() -> String {
    let mut pattern = ["[\\[{(](?:", BRACKET_ATOM, ")*[\\]})]"].concat();
    for _ in 0..NESTED_BRACKET_LEVELS {
        pattern = ["[\\[{(](?:", BRACKET_ATOM, "|", &pattern, ")*[\\]})]"].concat();
    }
    pattern
}

fn field_pattern_source(names: &[&str]) -> String {
    let escaped = names
        .iter()
        .map(|name| regex::escape(name))
        .collect::<Vec<_>>()
        .join("|");
    let bracketed = bracketed_value_pattern();
    format!(
        "(?i)(?P<field>(?:{escaped})=|['\"](?:{escaped})['\"]:\\s*)(?P<value>{QUOTED_VALUE}|{bracketed}|{BARE_VALUE})"
    )
}

fn key_pattern_source() -> String {
    let generated_suffix_length = GENERATED_KEY_LENGTH
        .saturating_mul(4)
        .saturating_add(2)
        .checked_div(3)
        .unwrap_or(0);
    let floor = MINIMUM_CUSTOM_KEY_LENGTH
        .saturating_sub(KEY_PREFIX.len())
        .min(generated_suffix_length);
    format!("{KEY_PREFIX}[A-Za-z0-9_-]{{{floor},}}")
}

pub(super) fn build(send_default_pii: bool) -> Result<Patterns, PatternError> {
    let mut field_names = fixture::DEFAULT_DENYLIST
        .iter()
        .chain(fixture::SENTRY_DENYLIST)
        .copied()
        .collect::<Vec<_>>();
    if !send_default_pii {
        field_names.extend(
            fixture::DEFAULT_PII_DENYLIST
                .iter()
                .chain(fixture::SENTRY_PII_DENYLIST)
                .copied(),
        );
    }
    let (email, sha256) = if send_default_pii {
        (None, None)
    } else {
        let email = Regex::new(EMAIL_BODY).map_err(|error| PatternError::Compile {
            reason: error.to_string(),
        })?;
        let sha256 = Regex::new(SHA256_BODY).map_err(|error| PatternError::Compile {
            reason: error.to_string(),
        })?;
        (Some(email), Some(sha256))
    };
    let field =
        Regex::new(&field_pattern_source(&field_names)).map_err(|error| PatternError::Compile {
            reason: error.to_string(),
        })?;
    let key = Regex::new(&key_pattern_source()).map_err(|error| PatternError::Compile {
        reason: error.to_string(),
    })?;
    Ok(Patterns {
        field,
        key,
        email,
        sha256,
    })
}

fn field_match_is_rejected(text: &str, start: usize, field: &str, value: &str) -> bool {
    let field_is_bare = field.ends_with('=');
    let before_byte = start
        .checked_sub(1)
        .and_then(|index| text.as_bytes().get(index));
    let boundary_violation = field_is_bare
        && before_byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
    let value_is_quoted_or_bracketed = value.starts_with(['\'', '"', '[', '{', '(']);
    let none_violation = !value_is_quoted_or_bracketed && value == "None";
    boundary_violation || none_violation
}

fn push_filtered_field(output: &mut String, field: &str, value_starts_with_quote: bool) {
    let quote = if value_starts_with_quote { '"' } else { '\'' };
    output.push_str(field);
    output.push(quote);
    output.push_str(FILTERED);
    output.push(quote);
}

fn advance_past_char(text: &str, start: usize) -> usize {
    let next = text.get(start..).and_then(|rest| rest.chars().next());
    next.map_or(text.len(), |ch| start.saturating_add(ch.len_utf8()))
}

fn scan_and_filter<M>(
    text: &str,
    mut next_match: impl FnMut(&str, usize) -> Option<(Range<usize>, bool, M)>,
    mut push_replacement: impl FnMut(&mut String, M),
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut copied_until = 0_usize;
    let mut scan_from = 0_usize;
    while scan_from <= text.len() {
        let Some((whole, rejected, data)) = next_match(text, scan_from) else {
            break;
        };
        if rejected {
            scan_from = advance_past_char(text, whole.start);
            continue;
        }
        output.push_str(text.get(copied_until..whole.start).unwrap_or_default());
        push_replacement(&mut output, data);
        copied_until = whole.end;
        scan_from = whole.end;
    }
    output.push_str(text.get(copied_until..).unwrap_or_default());
    output
}

fn scrub_fields(text: &str, field_pattern: &Regex) -> String {
    scan_and_filter(
        text,
        |haystack, scan_from| {
            let captures = field_pattern.captures_at(haystack, scan_from)?;
            let (whole, field, value) = (
                captures.get(0)?,
                captures.name("field")?,
                captures.name("value")?,
            );
            let rejected =
                field_match_is_rejected(haystack, whole.start(), field.as_str(), value.as_str());
            let data = (field.as_str().to_owned(), value.as_str().starts_with('"'));
            Some((whole.range(), rejected, data))
        },
        |output, (field, value_starts_with_quote)| {
            push_filtered_field(output, &field, value_starts_with_quote);
        },
    )
}

fn sha256_match_is_adjacent_to_word(text: &str, whole: regex::Match<'_>) -> bool {
    let before_byte = whole
        .start()
        .checked_sub(1)
        .and_then(|index| text.as_bytes().get(index));
    let after_byte = text.as_bytes().get(whole.end());
    before_byte.is_some_and(u8::is_ascii_alphanumeric)
        || after_byte.is_some_and(u8::is_ascii_alphanumeric)
}

fn scrub_sha256(text: &str, sha256_pattern: &Regex) -> String {
    scan_and_filter(
        text,
        |haystack, scan_from| {
            let whole = sha256_pattern.find_at(haystack, scan_from)?;
            let rejected = sha256_match_is_adjacent_to_word(haystack, whole);
            Some((whole.range(), rejected, ()))
        },
        |output, ()| output.push_str(FILTERED),
    )
}

pub(super) fn apply(patterns: &Patterns, text: &str) -> String {
    let fields_scrubbed = scrub_fields(text, &patterns.field);
    let keys_scrubbed = patterns
        .key
        .replace_all(&fields_scrubbed, FILTERED)
        .into_owned();
    let Some(email_pattern) = patterns.email.as_ref() else {
        return keys_scrubbed;
    };
    let emails_scrubbed = email_pattern
        .replace_all(&keys_scrubbed, FILTERED)
        .into_owned();
    let Some(sha256_pattern) = patterns.sha256.as_ref() else {
        return emails_scrubbed;
    };
    scrub_sha256(&emails_scrubbed, sha256_pattern)
}

pub fn scrub(text: &str, send_default_pii: bool) -> Result<String, PatternError> {
    let patterns = build(send_default_pii)?;
    Ok(apply(&patterns, text))
}

mod tests;
