use regex::{Captures, Match, Regex};

use super::pattern::{REDACTED, dotted_i};
use crate::error::RedactionError;

// Both path classes add `\x1c-\x1f` since Python's `\s` matches it but Rust's `\s` does not.
const UNIX_SYSTEM_PATH: &str =
    r#"/(?:etc|var|opt|usr|home|root|private|Users|tmp|mnt|srv)/[^\s\x1c-\x1f'")\]}>,]+"#;
const WINDOWS_DRIVE_PATH: &str = r#"[A-Za-z]:\\[^\s\x1c-\x1f'")\]}>,]+"#;
const PRIVATE_OR_LOOPBACK_IPV4_BODY: &str = r"(?:10(?:\.\d{1,3}){3}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2}|192\.168(?:\.\d{1,3}){2}|127(?:\.\d{1,3}){3})";
const INTERNAL_SUFFIX_HOSTNAME_BODY: &str =
    r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.(?:internal|local|corp|lan|intra|private)";

// Replays Python's `\b` without lookaround: Rust's `\b` treats a combining mark as a word character, Python's does not.
const PY_BOUNDARY_BEFORE: &str = r"(?:^|[^\p{L}\p{N}_])";
const PY_BOUNDARY_AFTER: &str = r"(?:$|[^\p{L}\p{N}_])";

const LAN_NET4: &str = "lan_net4";
const LAN_HOST: &str = "lan_host";

pub(super) struct InternalPatterns {
    whole: Regex,
}

pub(super) fn build() -> Result<InternalPatterns, RedactionError> {
    let inner = dotted_i(&format!(
        "{UNIX_SYSTEM_PATH}|{WINDOWS_DRIVE_PATH}\
         |{PY_BOUNDARY_BEFORE}(?P<{LAN_NET4}>{PRIVATE_OR_LOOPBACK_IPV4_BODY}){PY_BOUNDARY_AFTER}\
         |{PY_BOUNDARY_BEFORE}(?P<{LAN_HOST}>{INTERNAL_SUFFIX_HOSTNAME_BODY}){PY_BOUNDARY_AFTER}"
    ));
    let whole = Regex::new(&format!("(?i)(?:{inner})"))
        .map_err(|source| RedactionError::Pattern { source })?;
    Ok(InternalPatterns { whole })
}

impl InternalPatterns {
    // Mirrors SecretPatterns::replacement_span: the lan_net4/lan_host boundary stays out of the redaction.
    #[must_use]
    pub(super) fn redact(&self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        let mut copied_until = 0_usize;
        let mut scan_from = 0_usize;
        while scan_from <= text.len() {
            let Some(captures) = self.whole.captures_at(text, scan_from) else {
                break;
            };
            let Some(whole) = captures.get(0) else {
                break;
            };
            let (start, end) = Self::replacement_span(&captures, whole);
            output.push_str(text.get(copied_until..start).unwrap_or_default());
            output.push_str(REDACTED);
            copied_until = end;
            scan_from = end.max(scan_from.saturating_add(1));
        }
        output.push_str(text.get(copied_until..).unwrap_or_default());
        output
    }

    fn replacement_span(captures: &Captures<'_>, whole: Match<'_>) -> (usize, usize) {
        for name in [LAN_NET4, LAN_HOST] {
            if let Some(credential) = captures.name(name) {
                return (credential.start(), credential.end());
            }
        }
        (whole.start(), whole.end())
    }
}

mod tests;
