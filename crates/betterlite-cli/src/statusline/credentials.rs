use std::collections::BTreeMap;

use secrecy::SecretString;
use unicode_general_category::{GeneralCategory, get_general_category};

const CLAUDE_BASE_URL_KEYS: &[&str] = &["ANTHROPIC_BASE_URL"];
const CLAUDE_API_KEY_KEYS: &[&str] = &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"];
const CODEX_BASE_URL_KEYS: &[&str] = &["OPENAI_BASE_URL"];
const CODEX_API_KEY_KEYS: &[&str] = &["OPENAI_API_KEY"];
const VERSION_SUFFIX: &str = "/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
}
#[derive(Debug)]
pub struct Credentials {
    pub base_url: String,
    pub api_key: SecretString,
}
// An apiKeyHelper or keyring is never consulted: a status line reruns on every keystroke
pub(crate) fn resolve_credentials(
    environment: &BTreeMap<String, String>,
    agent: Agent,
) -> Option<Credentials> {
    let (base_url_keys, api_key_keys) = match agent {
        Agent::ClaudeCode => (CLAUDE_BASE_URL_KEYS, CLAUDE_API_KEY_KEYS),
        Agent::Codex => (CODEX_BASE_URL_KEYS, CODEX_API_KEY_KEYS),
    };
    let trimmed = first_value(environment, base_url_keys)
        .trim()
        .trim_end_matches('/');
    let base_url = trimmed.strip_suffix(VERSION_SUFFIX).unwrap_or(trimmed);
    let api_key = first_value(environment, api_key_keys).trim();
    if base_url.is_empty() || api_key.is_empty() {
        return None;
    }
    Some(Credentials {
        base_url: base_url.to_owned(),
        api_key: SecretString::from(api_key.to_owned()),
    })
}
fn first_value<'env>(environment: &'env BTreeMap<String, String>, keys: &[&str]) -> &'env str {
    keys.iter()
        .find_map(|key| environment.get(*key).filter(|value| !value.is_empty()))
        .map_or("", String::as_str)
}
// A control character in a label could redraw the screen or set the clipboard, so only printable text survives
pub(crate) fn printable(text: &str) -> String {
    text.chars()
        .filter(|character| {
            *character == ' '
                || !matches!(
                    get_general_category(*character),
                    GeneralCategory::Control
                        | GeneralCategory::Format
                        | GeneralCategory::Surrogate
                        | GeneralCategory::PrivateUse
                        | GeneralCategory::Unassigned
                        | GeneralCategory::LineSeparator
                        | GeneralCategory::ParagraphSeparator
                        | GeneralCategory::SpaceSeparator
                )
        })
        .collect()
}

mod tests;
