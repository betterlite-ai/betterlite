use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::agent_requests::endpoint::{AgentEndpoint, endpoint_url};
use crate::agent_requests::error::AgentRequestError;
use crate::agent_requests::prepare::{Headers, PreparedAgentCall};

pub const INSTRUCTIONS_KEY: &str = "instructions";
pub const SSL_VERIFY_KEY: &str = "ssl_verify";
pub const STREAM_ENABLED: bool = true;
pub const SYSTEM_ROLE: &str = "system";
pub const USER_ROLE: &str = "user";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadMessage {
    pub role: String,
    pub content: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadPayload {
    pub messages: Vec<ThreadMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamPayload {
    pub assistant_id: String,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Json>,
}

#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub url: String,
    pub headers: Headers,
    pub payload: StreamPayload,
    // LiteLLM hands this raw value to get_async_httpx_client, whose client B173 builds
    pub ssl_verify: Json,
}

// LiteLLM always POSTs the B559 create-thread-and-run URL, and only a falsey thread id sends messages
pub fn build_stream_request(
    prepared: &PreparedAgentCall,
    messages: &[Object],
    optional_params: &Object,
    litellm_params: &Object,
) -> Result<StreamRequest, AgentRequestError> {
    let url = endpoint_url(
        AgentEndpoint::ThreadAndRun,
        &prepared.api_base,
        &prepared.api_version,
    )?;
    let has_thread = prepared
        .thread_id
        .as_deref()
        .is_some_and(|thread_id| !thread_id.is_empty());
    let thread = if has_thread {
        None
    } else {
        Some(ThreadPayload {
            messages: thread_messages(messages),
        })
    };
    Ok(StreamRequest {
        url,
        headers: prepared.headers.clone(),
        payload: StreamPayload {
            assistant_id: prepared.agent_id.clone(),
            stream: STREAM_ENABLED,
            thread,
            instructions: optional_params.get(INSTRUCTIONS_KEY).cloned(),
        },
        ssl_verify: ssl_verify(litellm_params),
    })
}

fn thread_messages(messages: &[Object]) -> Vec<ThreadMessage> {
    let mut converted = Vec::new();
    for message in messages {
        let Some(role) = message.get("role").and_then(Json::as_str) else {
            continue;
        };
        if role != USER_ROLE && role != SYSTEM_ROLE {
            continue;
        }
        converted.push(ThreadMessage {
            role: String::from(USER_ROLE),
            content: message
                .get("content")
                .cloned()
                .unwrap_or_else(|| Json::Text(String::new())),
        });
    }
    converted
}

fn ssl_verify(litellm_params: &Object) -> Json {
    match litellm_params.get(SSL_VERIFY_KEY) {
        None | Some(Json::Null) => Json::Null,
        Some(value) => value.clone(),
    }
}
