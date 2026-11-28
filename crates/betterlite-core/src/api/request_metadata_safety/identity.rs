use betterlite_auth::master_key_admin::MASTER_KEY_ALIAS;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::api::request_metadata_safety::MetadataPrincipal;

const SHA256_HEX_LENGTH: usize = 64;
const EXPLICIT_TRACE_HEADER: &str = "x-litellm-trace-id";
const EXPLICIT_SESSION_HEADER: &str = "x-litellm-session-id";
const EXPLICIT_SESSION_HEADERS: [&str; 2] = [EXPLICIT_TRACE_HEADER, EXPLICIT_SESSION_HEADER];
const GENERIC_SESSION_PREFIX: &str = "x-";
const GENERIC_SESSION_SUFFIX: &str = "-session-id";
const BARE_SESSION_HEADER: &str = "x-session-id";
const MINIMUM_SESSION_ID_LENGTH: usize = 8;
const USER_AGENT_HEADER: &str = "user-agent";
const CODEX_PREFIX: &[u8] = b"codex";
const CODEX_SESSION_ID_HEADERS: [&str; 4] =
    ["session-id", "session_id", "thread-id", "conversation_id"];
const ANTHROPIC_SESSION_MARKER: &str = "_session_";
const USER_ID_KEY: &str = "user_id";
const SESSION_ID_KEY: &str = "session_id";

// only a key the proxy validated is stamped, and only in a shape UserAPIKeyAuth itself stores
#[must_use]
pub fn stampable_key_hash(principal: &MetadataPrincipal) -> Option<String> {
    if !principal.via_virtual_key {
        return None;
    }
    let api_key = principal.api_key.as_ref()?.expose_secret();
    if api_key == MASTER_KEY_ALIAS || is_sha256_hex(api_key) {
        return Some(api_key.to_owned());
    }
    None
}

// a user-controlled value written to one log line: newline and carriage return cannot forge another line
#[must_use]
pub fn sanitize_for_log(value: &Json) -> String {
    let text = match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    };
    text.replace(['\r', '\n'], "")
}

// the headers a chained call inherits: explicit ids first, then the vendor and client conventions
#[must_use]
pub fn get_chain_id_from_headers(headers: &IndexMap<String, String>) -> Option<String> {
    if headers.is_empty() {
        return None;
    }
    let mut normalized: IndexMap<String, String> = IndexMap::new();
    for (name, value) in headers {
        normalized.insert(name.to_lowercase(), value.clone());
    }
    normalized
        .get(EXPLICIT_TRACE_HEADER)
        .map(String::as_str)
        .or_else(|| normalized.get(EXPLICIT_SESSION_HEADER).map(String::as_str))
        .or_else(|| generic_session_id(&normalized))
        .or_else(|| codex_session_id(&normalized))
        .or_else(|| bare_session_id(&normalized))
        .map(str::to_owned)
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn anthropic_session_id_from_metadata(metadata: &Json) -> Option<String> {
    let Json::Object(fields) = metadata else {
        return None;
    };
    match fields.get(USER_ID_KEY) {
        Some(Json::Object(user)) => match user.get(SESSION_ID_KEY) {
            Some(Json::Text(session_id))
                if !session_id.is_empty() && is_session_id_text(session_id) =>
            {
                Some(session_id.clone())
            }
            Some(
                Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_)
                | Json::Object(_),
            )
            | None => None,
        },
        Some(Json::Text(user_id)) => session_id_after_marker(user_id),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => None,
    }
}

fn session_id_after_marker(user_id: &str) -> Option<String> {
    let marker_index = user_id.rfind(ANTHROPIC_SESSION_MARKER)?;
    let remainder = user_id.get(marker_index..)?;
    let session_id = remainder.strip_prefix(ANTHROPIC_SESSION_MARKER)?;
    if !session_id.is_empty() && is_session_id_text(session_id) {
        return Some(session_id.to_owned());
    }
    None
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == SHA256_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_valid_session_id(value: &str) -> bool {
    if value.len() < MINIMUM_SESSION_ID_LENGTH {
        return false;
    }
    is_session_id_text(value)
}

fn generic_session_id(normalized: &IndexMap<String, String>) -> Option<&str> {
    normalized.iter().find_map(|(name, value)| {
        if EXPLICIT_SESSION_HEADERS.contains(&name.as_str())
            || !is_generic_session_header(name)
            || !is_valid_session_id(value)
        {
            return None;
        }
        Some(value.as_str())
    })
}

fn codex_session_id(normalized: &IndexMap<String, String>) -> Option<&str> {
    let user_agent = normalized.get(USER_AGENT_HEADER)?;
    if !is_codex_user_agent(user_agent) {
        return None;
    }
    CODEX_SESSION_ID_HEADERS.iter().find_map(|header| {
        normalized
            .get(*header)
            .filter(|value| is_valid_session_id(value))
            .map(String::as_str)
    })
}

fn bare_session_id(normalized: &IndexMap<String, String>) -> Option<&str> {
    let value = normalized.get(BARE_SESSION_HEADER)?;
    if !is_valid_session_id(value) {
        return None;
    }
    Some(value.as_str())
}

fn is_generic_session_header(name: &str) -> bool {
    let Some(middle) = name
        .strip_prefix(GENERIC_SESSION_PREFIX)
        .and_then(|rest| rest.strip_suffix(GENERIC_SESSION_SUFFIX))
    else {
        return false;
    };
    !middle.is_empty() && !middle.contains('\n')
}

fn is_session_id_text(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn is_codex_user_agent(user_agent: &str) -> bool {
    let bytes = user_agent.as_bytes();
    let Some(prefix) = bytes.get(..CODEX_PREFIX.len()) else {
        return false;
    };
    prefix.eq_ignore_ascii_case(CODEX_PREFIX)
        && matches!(
            bytes.get(CODEX_PREFIX.len()),
            Some(b'-' | b'_' | b' ' | b'/')
        )
}
