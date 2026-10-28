//! Field and value matching for `sentry_scrubbing.py`'s repr scrubber.
//!
//! The `regex` crate refuses lookaround (it would break its linear-time guarantee),
//! so the two lookaround spots in the Python source (a field-name word boundary and
//! a "don't swallow bare `None`" guard) are replicated here as O(1) byte checks after
//! an un-anchored match, with the scan cursor stepping forward by one char on a
//! rejected candidate instead of the match length. Everything else -- the quoted,
//! bracketed and bare value alternatives, three levels of nested brackets -- compiles
//! to plain regex exactly as `sentry_scrubbing.py` builds it.
//!
//! Compiled patterns are never cached behind a global: callers build a [`Patterns`]
//! once (through [`build`]) and pass it down to every string they scrub.

use regex::Regex;

use super::fixture::{
    self, KEY_PREFIX, LENGTH_OF_LITELLM_GENERATED_KEY, MINIMUM_CUSTOM_KEY_LENGTH,
};

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
    let generated_suffix_length = LENGTH_OF_LITELLM_GENERATED_KEY
        .saturating_mul(4)
        .saturating_add(2)
        .checked_div(3)
        .unwrap_or(0);
    let floor = MINIMUM_CUSTOM_KEY_LENGTH
        .saturating_sub(KEY_PREFIX.len())
        .min(generated_suffix_length);
    format!("{KEY_PREFIX}[A-Za-z0-9_-]{{{floor},}}")
}

fn compile(source: &str) -> Result<Regex, PatternError> {
    Regex::new(source).map_err(|error| PatternError::Compile {
        reason: error.to_string(),
    })
}

/// Builds the compiled patterns for one `send_default_pii` setting, mirroring
/// `build_string_scrubber`. The caller owns the result and passes it to [`apply`]
/// for every string it scrubs, instead of recompiling per call.
pub(super) fn build(send_default_pii: bool) -> Result<Patterns, PatternError> {
    let mut field_names = fixture::secret_field_names();
    if !send_default_pii {
        field_names.extend(fixture::pii_field_names());
    }
    let (email, sha256) = if send_default_pii {
        (None, None)
    } else {
        (Some(compile(EMAIL_BODY)?), Some(compile(SHA256_BODY)?))
    };
    Ok(Patterns {
        field: compile(&field_pattern_source(&field_names))?,
        key: compile(&key_pattern_source())?,
        email,
        sha256,
    })
}

fn advance_past_char(text: &str, start: usize) -> usize {
    text.get(start..)
        .and_then(|rest| rest.chars().next())
        .map_or(text.len(), |ch| start.saturating_add(ch.len_utf8()))
}

const fn is_name_boundary_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn field_match_is_rejected(text: &str, start: usize, field: &str, value: &str) -> bool {
    let field_is_bare = field.ends_with('=');
    let before_byte = start
        .checked_sub(1)
        .and_then(|index| text.as_bytes().get(index));
    let boundary_violation =
        field_is_bare && before_byte.is_some_and(|byte| is_name_boundary_byte(*byte));
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

fn scrub_fields(text: &str, field_pattern: &Regex) -> String {
    let mut output = String::with_capacity(text.len());
    let mut copied_until = 0_usize;
    let mut scan_from = 0_usize;
    while scan_from <= text.len() {
        let Some(captures) = field_pattern.captures_at(text, scan_from) else {
            break;
        };
        let (Some(whole), Some(field), Some(value)) = (
            captures.get(0),
            captures.name("field"),
            captures.name("value"),
        ) else {
            break;
        };
        if field_match_is_rejected(text, whole.start(), field.as_str(), value.as_str()) {
            scan_from = advance_past_char(text, whole.start());
            continue;
        }
        output.push_str(text.get(copied_until..whole.start()).unwrap_or_default());
        push_filtered_field(&mut output, field.as_str(), value.as_str().starts_with('"'));
        copied_until = whole.end();
        scan_from = whole.end();
    }
    output.push_str(text.get(copied_until..).unwrap_or_default());
    output
}

fn scrub_sha256(text: &str, sha256_pattern: &Regex) -> String {
    let mut output = String::with_capacity(text.len());
    let mut copied_until = 0_usize;
    let mut scan_from = 0_usize;
    while scan_from <= text.len() {
        let Some(whole) = sha256_pattern.find_at(text, scan_from) else {
            break;
        };
        let before_byte = whole
            .start()
            .checked_sub(1)
            .and_then(|index| text.as_bytes().get(index));
        let after_byte = text.as_bytes().get(whole.end());
        let adjacent_to_word = before_byte.is_some_and(u8::is_ascii_alphanumeric)
            || after_byte.is_some_and(u8::is_ascii_alphanumeric);
        if adjacent_to_word {
            scan_from = advance_past_char(text, whole.start());
            continue;
        }
        output.push_str(text.get(copied_until..whole.start()).unwrap_or_default());
        output.push_str(FILTERED);
        copied_until = whole.end();
        scan_from = whole.end();
    }
    output.push_str(text.get(copied_until..).unwrap_or_default());
    output
}

/// Scrubs `text` with an already-built [`Patterns`], mirroring `build_string_scrubber`'s
/// closure: the field-name repr pass first, then the key/email/sha256 value passes.
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

/// Builds patterns for one call and scrubs `text`, for callers that only need a
/// single string scrubbed. `scrub_event` builds [`Patterns`] once instead, since it
/// scrubs many strings from the same tree.
pub(super) fn scrub(text: &str, send_default_pii: bool) -> Result<String, PatternError> {
    let patterns = build(send_default_pii)?;
    Ok(apply(&patterns, text))
}

mod tests;
