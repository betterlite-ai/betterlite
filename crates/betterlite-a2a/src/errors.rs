use thiserror::Error;

use crate::card_resolver::fetch_error::ProbeFailure;
use attempts::ProbeAttempts;
use retries::RetryCounts;

pub mod attempts;
pub mod retries;
mod tests;

pub const LLM_PROVIDER: &str = "a2a_agent";

#[derive(Debug, Error)]
pub enum A2AError {
    #[error("{message}{}", .retries.render())]
    Base {
        message: String,
        model: Option<String>,
        debug_info: Option<String>,
        retries: RetryCounts,
    },
    #[error("{message}{}", .retries.render())]
    Connection {
        message: String,
        url: Option<String>,
        model: Option<String>,
        debug_info: Option<String>,
        retries: RetryCounts,
    },
    #[error("{message}")]
    AgentCard {
        message: String,
        url: Option<String>,
        status_code: u16,
    },
    #[error("no agent card found at {base_url}. Tried {}", .attempts.render())]
    AgentCardDiscovery {
        base_url: String,
        attempts: ProbeAttempts,
        status_code: u16,
    },
    #[error("the agent card points at the internal URL '{localhost_url}'; retry with '{base_url}'")]
    LocalhostUrl {
        localhost_url: String,
        base_url: String,
        model: Option<String>,
        #[source]
        original_error: Option<Box<Self>>,
    },
}

impl A2AError {
    // The first failure that is not a 404 wins, and 404 stands when every probe was missing.
    #[must_use]
    pub fn agent_card_discovery(base_url: String, failures: Vec<ProbeFailure>) -> Self {
        let status_code = failures
            .iter()
            .filter_map(|failure| failure.reason.status_code())
            .find(|status| *status != 404)
            .unwrap_or(404);
        Self::AgentCardDiscovery {
            base_url,
            attempts: ProbeAttempts { failures },
            status_code,
        }
    }

    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Base { .. } => 500,
            Self::Connection { .. } | Self::LocalhostUrl { .. } => 503,
            Self::AgentCard { status_code, .. } | Self::AgentCardDiscovery { status_code, .. } => {
                *status_code
            }
        }
    }

    #[must_use]
    pub fn url(&self) -> Option<&str> {
        match self {
            Self::Base { .. } => None,
            Self::Connection { url, .. } | Self::AgentCard { url, .. } => url.as_deref(),
            Self::AgentCardDiscovery { base_url, .. } => Some(base_url.as_str()),
            Self::LocalhostUrl { localhost_url, .. } => Some(localhost_url.as_str()),
        }
    }
}
