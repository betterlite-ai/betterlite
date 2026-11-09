use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::agents::json::AgentJson;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::UserUrlError;
use serde::Deserialize;
use thiserror::Error;

// The current A2A spec path, the previous one, then the root fallback some agents serve
pub const AGENT_CARD_WELL_KNOWN_PATHS: [&str; 3] = [
    "/.well-known/agent-card.json",
    "/.well-known/agent.json",
    "/agent.json",
];

pub const DEFAULT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum DiscoveryMode {
    #[default]
    WellKnownFallback,
    LanggraphPlatform,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AgentCardDiscoveryError {
    #[error("base_url is required")]
    MissingBaseUrl,
    #[error("langgraph_platform discovery requires params.assistant_id")]
    MissingAssistantId,
    #[error("Could not fetch agent card from {base_url} (mode={mode}). Last error: {last_error}")]
    AllPathsFailed {
        base_url: String,
        mode: DiscoveryMode,
        last_error: String,
    },
}

#[derive(Debug, Error)]
pub enum CardTransportError {
    #[error(transparent)]
    Url(#[from] UserUrlError),
    #[error(transparent)]
    Read(#[from] TransportError),
    #[error("the request did not finish within {timeout:?}")]
    Deadline { timeout: Duration },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CardResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
enum PathFailure {
    #[error("{url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: CardTransportError,
    },
    #[error("{url}: HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("{url}: invalid JSON ({source})")]
    Json {
        url: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{url}: expected a JSON object, got {kind}")]
    NotObject { url: String, kind: &'static str },
}

#[async_trait]
pub trait CardSource: Send + Sync {
    async fn get(&self, url: &str) -> Result<CardResponse, CardTransportError>;
}

pub async fn fetch_well_known_card<S>(
    source: &S,
    base_url: &str,
    discovery_mode: DiscoveryMode,
    params: &BTreeMap<String, AgentJson>,
) -> Result<AgentJson, AgentCardDiscoveryError>
where
    S: CardSource + ?Sized,
{
    if base_url.is_empty() {
        return Err(AgentCardDiscoveryError::MissingBaseUrl);
    }
    let normalized = base_url.trim_end_matches('/');
    let paths = match discovery_mode {
        DiscoveryMode::WellKnownFallback => AGENT_CARD_WELL_KNOWN_PATHS
            .iter()
            .map(|path| (*path).to_owned())
            .collect(),
        DiscoveryMode::LanggraphPlatform => langgraph_paths(params)?,
    };
    let mut last_error: Option<PathFailure> = None;
    for path in &paths {
        let url = format!("{normalized}{path}");
        match card_from(&url, source).await {
            Ok(card) => {
                tracing::debug!(url, "A2A discovery succeeded");
                return Ok(card);
            }
            Err(error) => {
                tracing::debug!(url = %url, error = %error, "A2A discovery path failed");
                last_error = Some(error);
            }
        }
    }
    Err(AgentCardDiscoveryError::AllPathsFailed {
        base_url: base_url.to_owned(),
        mode: discovery_mode,
        last_error: last_error.map_or_else(String::new, |failure| failure.to_string()),
    })
}

fn langgraph_paths(
    params: &BTreeMap<String, AgentJson>,
) -> Result<Vec<String>, AgentCardDiscoveryError> {
    let assistant_id = match params.get("assistant_id") {
        None | Some(AgentJson::Null) => {
            return Err(AgentCardDiscoveryError::MissingAssistantId);
        }
        Some(value) => assistant_id_text(value),
    };
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("assistant_id", &assistant_id)
        .finish();
    Ok(AGENT_CARD_WELL_KNOWN_PATHS
        .iter()
        .map(|path| format!("{path}?{query}"))
        .collect())
}

// Python's urlencode stringifies anything; JSON text keeps a scalar as sent and a list as JSON, not as repr
fn assistant_id_text(value: &AgentJson) -> String {
    match value {
        AgentJson::String(text) => text.clone(),
        AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => value.to_string(),
    }
}

async fn card_from<S>(url: &str, transport: &S) -> Result<AgentJson, PathFailure>
where
    S: CardSource + ?Sized,
{
    let response = match transport.get(url).await {
        Ok(response) => response,
        Err(source) => {
            return Err(PathFailure::Transport {
                url: url.to_owned(),
                source,
            });
        }
    };
    if response.status >= 400 {
        return Err(PathFailure::Status {
            url: url.to_owned(),
            status: response.status,
        });
    }
    let card: AgentJson = match serde_json::from_slice(&response.body) {
        Ok(card) => card,
        Err(source) => {
            return Err(PathFailure::Json {
                url: url.to_owned(),
                source,
            });
        }
    };
    if !card.is_object() {
        return Err(PathFailure::NotObject {
            url: url.to_owned(),
            kind: json_kind(&card),
        });
    }
    Ok(card)
}

const fn json_kind(value: &AgentJson) -> &'static str {
    match value {
        AgentJson::Null => "null",
        AgentJson::Bool(_) => "a boolean",
        AgentJson::Number(_) => "a number",
        AgentJson::String(_) => "a string",
        AgentJson::Array(_) => "an array",
        AgentJson::Object(_) => "an object",
    }
}
