use crate::error::RedactionError;
use internal::InternalPatterns;
use pattern::SecretPatterns;

const TRACEBACK_MARKER: &str = "Traceback (most recent call last):";

// CPython's _PyUnicode_IsWhitespace table (Objects/unicodetype_db.h); char::is_whitespace() misses U+001C-U+001F.
const PYTHON_WHITESPACE: [char; 29] = [
    '\t', '\n', '\u{b}', '\u{c}', '\r', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ' ', '\u{85}',
    '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}',
    '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}',
    '\u{205f}', '\u{3000}',
];

// litellm/constants.py's MINIMUM_CUSTOM_KEY_LENGTH default; the config layer resolves the env var.
pub const MINIMUM_CUSTOM_KEY_LENGTH: usize = 16;

// Holds the compiled patterns so callers inject one instance instead of a lazy global.
pub struct Redactor {
    secret: SecretPatterns,
    internal_detail: InternalPatterns,
}

impl Redactor {
    pub fn new(minimum_custom_key_length: usize) -> Result<Self, RedactionError> {
        let secret = pattern::build(minimum_custom_key_length)?;
        let internal_detail = internal::build()?;
        Ok(Self {
            secret,
            internal_detail,
        })
    }

    #[must_use]
    pub fn redact_string(&self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        let mut copied_until = 0_usize;
        let mut scan_from = 0_usize;
        while scan_from <= text.len() {
            let Some(captures) = self.secret.whole.captures_at(text, scan_from) else {
                break;
            };
            let Some(whole) = captures.get(0) else {
                break;
            };
            let Some((replace_start, replace_end)) =
                self.secret.replacement_span(text, &captures, whole)
            else {
                // The boundary-guarded group's `\b` check rejected this candidate and no later-ranked branch claims its start, so it is not a secret: resume just past it, unredacted.
                let step = text
                    .get(whole.start()..)
                    .and_then(|rest| rest.chars().next())
                    .map_or(1, char::len_utf8);
                scan_from = whole
                    .start()
                    .saturating_add(step)
                    .max(scan_from.saturating_add(1));
                continue;
            };
            output.push_str(text.get(copied_until..replace_start).unwrap_or_default());
            output.push_str(pattern::REDACTED);
            copied_until = replace_end;
            // A stronger earlier-ranked match can reach past the whole match, so scan resumes there too.
            scan_from = replace_end.max(scan_from.saturating_add(1));
        }
        output.push_str(text.get(copied_until..).unwrap_or_default());
        output
    }

    #[must_use]
    pub fn redact_internal_details(&self, text: &str) -> String {
        let without_traceback = text.find(TRACEBACK_MARKER).map_or(text, |index| {
            text.get(..index)
                .unwrap_or(text)
                .trim_end_matches(PYTHON_WHITESPACE)
        });
        let secret_scrubbed = self.redact_string(without_traceback);
        self.internal_detail.redact(&secret_scrubbed)
    }

    #[must_use]
    pub fn redact_structured_value(&self, key: Option<&str>, value: &str) -> String {
        let scrubbed = self.redact_string(value);
        let Some(key_name) = key else {
            return scrubbed;
        };
        if scrubbed != value {
            return scrubbed;
        }
        let rendered = format!("'{key_name}': '{value}'");
        if self.redact_string(&rendered) == rendered {
            value.to_owned()
        } else {
            pattern::REDACTED.to_owned()
        }
    }
}

mod internal;
mod pattern;
mod tests;
