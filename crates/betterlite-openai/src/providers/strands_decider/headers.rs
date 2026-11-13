use betterlite_models::decisions::{Extra, Json};

use crate::openai_base_config::Headers;

use super::arn::AgentCoreRuntime;

pub const SESSION_HEADER: &str = "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id";
const SESSION_PARAMETER: &str = "agentcore_runtime_session_id";
const HOST_HEADER: &str = "host";
const RESERVED_PREFIXES: [&str; 2] = ["x-amzn-bedrock-agentcore-runtime-", "x-amz-"];

pub struct RuntimeHeaders {
    pub headers: Headers,
    pub dropped: Vec<String>,
}

#[must_use]
pub fn runtime_headers(
    headers: &Headers,
    runtime: &AgentCoreRuntime,
    litellm_params: &Extra,
) -> RuntimeHeaders {
    let mut kept = Headers::new();
    let mut dropped = Vec::new();
    for (name, value) in headers {
        if is_reserved(name) {
            dropped.push(name.clone());
        } else {
            kept.insert(name.clone(), value.clone());
        }
    }
    if !dropped.is_empty() {
        tracing::warn!(dropped = ?dropped, "not forwarding reserved AgentCore headers");
    }
    let session_id = configured_session_id(litellm_params)
        .map_or_else(|| runtime.default_session_id(), str::to_owned);
    kept.insert(SESSION_HEADER.to_owned(), session_id);
    RuntimeHeaders {
        headers: kept,
        dropped,
    }
}

fn is_reserved(name: &str) -> bool {
    let lowered = name.to_lowercase();
    lowered == HOST_HEADER
        || RESERVED_PREFIXES
            .iter()
            .any(|prefix| lowered.starts_with(*prefix))
}

fn configured_session_id(litellm_params: &Extra) -> Option<&str> {
    match litellm_params.get(SESSION_PARAMETER) {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text),
        Some(_) => {
            tracing::error!(
                parameter = SESSION_PARAMETER,
                "the configured session id is not text"
            );
            None
        }
    }
}
