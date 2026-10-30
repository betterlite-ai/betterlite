use regex::{Captures, Match, Regex, RegexBuilder};

use crate::error::RedactionError;

pub(super) const REDACTED: &str = "REDACTED";

// Every class below adds `\x1c-\x1f` since Python's `\s` matches it but Rust's `\s` does not.
const STOP_CLASS: &str = r#"[^\s\x1c-\x1f,'"})\]{}>]"#;
const KEY_VALUE_SEPARATOR: &str = r#"['"]?[\s\x1c-\x1f]*[:=][\s\x1c-\x1f]*['"]?"#;
const SK_PREFIX: &str = "sk-";

// Python's `\w` also matches Unicode letters and numbers (e.g. No-category digits) that Rust's `\w` misses.
const PY_WORD: &str = r"\p{L}\p{N}_";

const PEM_BLOCK: &str =
    r"-----BEGIN[A-Z \-]*PRIVATE KEY-----[\s\S]*?-----END[A-Z \-]*PRIVATE KEY-----";
const YA29_BODY: &str = r"ya29\.[A-Za-z0-9_.~+/-]+";
const AKIA: &str = "(?:AKIA|ASIA)[0-9A-Z]{16}";
const BEARER: &str = r"Bearer[\s\x1c-\x1f]+[A-Za-z0-9\-._~+/]{10,}=*";
const BASIC: &str = r"Basic[\s\x1c-\x1f]+[A-Za-z0-9+/]{10,}={0,2}";
const X_AK: &str = r"x-ak-[A-Za-z0-9\-_]{20,}";
const AIZA: &str = r"AIza[0-9A-Za-z\-_]{35}";
const DAPI: &str = "dapi[0-9a-f]{32}";
const JWT_BODY: &str = r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*";
const SERVICE_ACCOUNT: &str =
    r#"\{[^{}]*"type"[\s\x1c-\x1f]*:[\s\x1c-\x1f]*"service_account"[^{}]*(?:\{[^{}]*\}[^{}]*)*\}"#;
const MASTER_KEY_NAMES: &str = concat!(
    "master_key|xai_key|database_url|db_url|connection_string|",
    "aws_secret_access_key|aws_session_token|aws_access_key_id|s3_secret_access_key|s3_access_key_id|",
    "signing_key|encryption_key|auth_token|access_token|refresh_token|",
    "slack_webhook_url|webhook_url|database_connection_string|huggingface_token|jwt_secret",
);

// Index of each guarded branch inside branches(), in LiteLLM's priority order.
const URL_QUERY_CRED: (&str, usize) = ("url_query_cred", 7);
const URL_KEY: (&str, usize) = ("url_key", 12);
const DB_CONN: (&str, usize) = ("db_conn", 14);
// Named "azure_sas" rather than LiteLLM's "sig" wording so dotted_i() can fold the whole branch without touching the group name.
const AZURE_SAS: (&str, usize) = ("azure_sas", 20);
// The byte before db_conn's credential is always "/" from "://", so url_query_cred's and url_key's "[?&]" guard can never win there.
const DB_CONN_SKIPS: [usize; 2] = [URL_QUERY_CRED.1, URL_KEY.1];
const GUARDS: [(&str, usize, &[usize]); 4] = [
    (URL_QUERY_CRED.0, URL_QUERY_CRED.1, &[]),
    (URL_KEY.0, URL_KEY.1, &[]),
    (DB_CONN.0, DB_CONN.1, &DB_CONN_SKIPS),
    (AZURE_SAS.0, AZURE_SAS.1, &[]),
];

// Groups whose Python pattern leads with `\b`, not a consuming lookaround Rust's regex crate can't express; the boundary is checked against the original text in code instead, paired with the branch's rank in branches() for replaying the branches LiteLLM ranks after it.
const YA29_TOKEN: (&str, usize) = ("ya29_token", 1);
const JWT_TOKEN: (&str, usize) = ("jwt_token", 19);
const BOUNDARY_GUARDED: [(&str, usize); 2] = [YA29_TOKEN, JWT_TOKEN];

// Folds U+0130/U+0131 onto i/I and onto every bare A-Z/a-z range, since Rust's (?i) folding leaves out Python's re.IGNORECASE mapping for the dotted and dotless I; none of these patterns negates a class spanning i or I, so a plain substring swap is safe.
pub(super) fn dotted_i(pattern: &str) -> String {
    let widened = pattern
        .replace("A-Z", "A-Z\u{130}\u{131}")
        .replace("a-z", "a-z\u{130}\u{131}");
    let mut folded = String::with_capacity(widened.len());
    for ch in widened.chars() {
        if ch == 'i' || ch == 'I' {
            folded.push_str("[i\u{130}\u{131}]");
        } else {
            folded.push(ch);
        }
    }
    folded
}

// Matches Python's `\w` set (`\p{L}\p{N}_`), which excludes combining marks that char::is_alphanumeric() treats as alphabetic.
fn is_py_word_char(py_word: &Regex, ch: char) -> bool {
    let mut buf = [0_u8; 4];
    py_word.is_match(ch.encode_utf8(&mut buf))
}

fn branches(minimum_custom_key_length: usize) -> Vec<String> {
    let sk_min_len = minimum_custom_key_length.saturating_sub(SK_PREFIX.len());
    let raw = vec![
        PEM_BLOCK.to_owned(),
        format!("(?P<ya29_token>{YA29_BODY})"),
        format!(r"(?:client_secret|azure_password|azure_username)[\s\x1c-\x1f]+{STOP_CLASS}+"),
        AKIA.to_owned(),
        BEARER.to_owned(),
        BASIC.to_owned(),
        format!(r"{SK_PREFIX}[A-Za-z0-9\-_]{{{sk_min_len},}}"),
        format!(
            r#"[?&](?P<url_query_cred>(?:api[_-]?key|[{PY_WORD}]*(?:token|password|passwd|client_secret|secret_key|_secret))=[^\s\x1c-\x1f&'"]+)"#
        ),
        format!("(?:api[_-]?key){KEY_VALUE_SEPARATOR}{STOP_CLASS}{{8,}}"),
        format!("(?:x-api-key|api-key){KEY_VALUE_SEPARATOR}{STOP_CLASS}+"),
        X_AK.to_owned(),
        AIZA.to_owned(),
        r#"[?&](?P<url_key>key=[^\s\x1c-\x1f&'"]{8,})"#.to_owned(),
        format!(
            r"\b[{PY_WORD}]*(?:password|passwd|client_secret|secret_key|_secret){KEY_VALUE_SEPARATOR}{STOP_CLASS}+"
        ),
        // Bounded like LiteLLM's lookaround window: up to 4096 bytes either side of the ":" separator.
        r#"://(?P<db_conn>[^\s\x1c-\x1f'":]{0,4096}:[^\s\x1c-\x1f'"]{1,4096})@"#.to_owned(),
        DAPI.to_owned(),
        format!(r"litellm\.[A-Za-z0-9_]*_key{KEY_VALUE_SEPARATOR}{STOP_CLASS}+"),
        format!("private_key{KEY_VALUE_SEPARATOR}(?:{PEM_BLOCK}|{STOP_CLASS}+)"),
        format!("(?:{MASTER_KEY_NAMES}){KEY_VALUE_SEPARATOR}{STOP_CLASS}+"),
        format!("(?P<jwt_token>{JWT_BODY})"),
        "[?&](?P<azure_sas>sig=[A-Za-z0-9%+/=]+)".to_owned(),
        SERVICE_ACCOUNT.to_owned(),
    ];
    raw.into_iter().map(|branch| dotted_i(&branch)).collect()
}

// db_conn's two 4096-byte bounded classes unroll past the crate's 10 MiB default program size.
const COMPILED_PROGRAM_SIZE_LIMIT: usize = 64 * 1024 * 1024;

pub(super) struct SecretPatterns {
    pub(super) whole: Regex,
    // For each guarded group, the branches LiteLLM ranks ahead of it, anchored to compete for the same start.
    priority_before: Vec<(&'static str, Regex)>,
    // For each boundary-guarded group, the branches LiteLLM ranks after it, anchored to compete for the same start once the `\b` check rejects the guarded group's own match.
    boundary_guarded: Vec<(&'static str, Regex)>,
    py_word: Regex,
}

// Builds one anchored regex per (name, branch subset), joined and compiled as `^(?:...)` to re-run at a credential's start, letting the named branches compete for that same start.
fn anchored_regexes<'branch>(
    selections: impl IntoIterator<Item = (&'static str, Vec<&'branch str>)>,
) -> Result<Vec<(&'static str, Regex)>, RedactionError> {
    let mut patterns = Vec::new();
    for (name, selected) in selections {
        let joined = selected.join("|");
        let regex = RegexBuilder::new(&format!("(?i)^(?:{joined})"))
            .size_limit(COMPILED_PROGRAM_SIZE_LIMIT)
            .build()
            .map_err(|source| RedactionError::Pattern { source })?;
        patterns.push((name, regex));
    }
    Ok(patterns)
}

pub(super) fn build(minimum_custom_key_length: usize) -> Result<SecretPatterns, RedactionError> {
    let branches = branches(minimum_custom_key_length);
    let whole = RegexBuilder::new(&format!("(?i)(?:{})", branches.join("|")))
        .size_limit(COMPILED_PROGRAM_SIZE_LIMIT)
        .build()
        .map_err(|source| RedactionError::Pattern { source })?;
    let priority_before = anchored_regexes(GUARDS.iter().map(|(name, rank, skips)| {
        let selected = branches
            .get(..*rank)
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(index, _)| !skips.contains(index))
            .map(|(_, branch)| branch.as_str())
            .collect();
        (*name, selected)
    }))?;
    let boundary_guarded = anchored_regexes(BOUNDARY_GUARDED.iter().map(|(name, rank)| {
        let selected = branches
            .get(rank.saturating_add(1)..)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        (*name, selected)
    }))?;
    let py_word = Regex::new(&format!("^[{PY_WORD}]$"))
        .map_err(|source| RedactionError::Pattern { source })?;
    Ok(SecretPatterns {
        whole,
        priority_before,
        boundary_guarded,
        py_word,
    })
}

impl SecretPatterns {
    // Replays LiteLLM's priority order at the credential's real start, and returns None when a boundary-guarded group's `\b` check fails with no later-ranked branch claiming the same start, meaning the candidate is not a secret.
    pub(super) fn replacement_span(
        &self,
        text: &str,
        captures: &Captures<'_>,
        whole: Match<'_>,
    ) -> Option<(usize, usize)> {
        for (name, earlier) in &self.priority_before {
            let Some(credential) = captures.name(name) else {
                continue;
            };
            let remainder = text.get(credential.start()..).unwrap_or_default();
            let stronger_end = earlier
                .find(remainder)
                .map(|found| credential.start().saturating_add(found.end()))
                .filter(|end| *end >= credential.end());
            return Some((
                credential.start(),
                stronger_end.unwrap_or_else(|| credential.end()),
            ));
        }
        for (name, later) in &self.boundary_guarded {
            let Some(credential) = captures.name(name) else {
                continue;
            };
            let preceded_by_word_char = text
                .get(..credential.start())
                .and_then(|prefix| prefix.chars().next_back())
                .is_some_and(|ch| is_py_word_char(&self.py_word, ch));
            if !preceded_by_word_char {
                return Some((credential.start(), credential.end()));
            }
            let remainder = text.get(credential.start()..).unwrap_or_default();
            return later.find(remainder).map(|found| {
                (
                    credential.start(),
                    credential.start().saturating_add(found.end()),
                )
            });
        }
        Some((whole.start(), whole.end()))
    }
}

mod tests;
