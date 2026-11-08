use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::steering::truthiness;

const CODEX_SESSION_KEYS: [&str; 4] = ["session-id", "session_id", "thread-id", "conversation_id"];

// litellm.integrations.langfuse._is_session_header_trace
#[must_use]
pub fn is_session_header_trace(
    trace_id: Option<&Json>,
    session_id: Option<&Json>,
    proxy_server_request: Option<&Json>,
) -> bool {
    let (Some(Json::Text(trace)), Some(Json::Text(session))) = (trace_id, session_id) else {
        return false;
    };
    let Some(Json::Object(request)) = proxy_server_request else {
        return false;
    };
    let Some(Json::Object(raw_headers)) = request.get("headers") else {
        return false;
    };
    let headers: IndexMap<String, &Json> = raw_headers
        .iter()
        .map(|(key, value)| (key.to_lowercase(), value))
        .collect();
    session_header_matches(trace, session, &headers)
}

fn session_header_matches(
    trace_id: &str,
    session_id: &str,
    headers: &IndexMap<String, &Json>,
) -> bool {
    if headers
        .get("x-litellm-trace-id")
        .copied()
        .is_some_and(truthiness)
    {
        return false;
    }
    if headers
        .get("langfuse_trace_id")
        .copied()
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return false;
    }
    let same_session = header_text(headers, "langfuse_session_id") == Some(session_id);
    if trace_id != session_id && !same_session {
        return false;
    }
    if header_text(headers, "x-litellm-session-id") == Some(trace_id) {
        return true;
    }
    if !is_trace_id(trace_id) {
        return false;
    }
    let codex = user_agent_is_codex(headers.get("user-agent").copied());
    headers.iter().any(|(key, value)| {
        matches!(*value, Json::Text(text) if text.as_str() == trace_id)
            && (key == "x-session-id"
                || is_custom_session_header(key)
                || (codex && CODEX_SESSION_KEYS.contains(&key.as_str())))
    })
}

fn is_trace_id(candidate: &str) -> bool {
    if candidate.len() < 8 {
        return false;
    }
    candidate
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

fn is_custom_session_header(key: &str) -> bool {
    key.strip_prefix("x-")
        .and_then(|rest| rest.strip_suffix("-session-id"))
        .is_some_and(|middle| !middle.is_empty())
}

fn user_agent_is_codex(user_agent: Option<&Json>) -> bool {
    let Some(Json::Text(agent)) = user_agent else {
        return false;
    };
    agent
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("codex"))
        && agent
            .chars()
            .nth(5)
            .is_some_and(|separator| matches!(separator, '-' | '_' | ' ' | '/'))
}

fn header_text<'value>(
    headers: &IndexMap<String, &'value Json>,
    name: &str,
) -> Option<&'value str> {
    match headers.get(name).copied() {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
