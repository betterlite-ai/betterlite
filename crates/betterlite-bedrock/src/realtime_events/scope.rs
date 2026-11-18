use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PENDING_SESSION_UPDATE_KEY: &str = "litellm.bedrock_realtime.pending_session_update";
pub const SESSION_COMMITTED_KEY: &str = "litellm.bedrock_realtime.session_committed";
pub const COMMITTED_FAILURE_KEY: &str = "litellm.bedrock_realtime.committed_failure";

const COMMITTED_MESSAGE: &str =
    "Bedrock realtime session already committed to a provider stream; it cannot be replayed";

// The ASGI scope outlives one bridge attempt, so an attempted replay reads what the failed attempt left behind
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct EventScope {
    pub pending_session_update: Json,
    pub session_committed: Json,
    pub committed_failure: Json,
}

// An absent scope key reads as null; only true and a string session update mean anything to the bridge
impl Default for EventScope {
    fn default() -> Self {
        Self {
            pending_session_update: Json::Null,
            session_committed: Json::Null,
            committed_failure: Json::Null,
        }
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct SessionCommitted {
    pub message: String,
    pub status_code: u16,
}

impl EventScope {
    // Only the identity True commits: the scope is a shared mapping, and 1 or "true" are not that marker
    pub fn pending_session_update(&self) -> Result<Option<String>, SessionCommitted> {
        if matches!(self.session_committed, Json::Bool(true)) {
            let suffix = match &self.committed_failure {
                Json::Text(failure) if !failure.is_empty() => {
                    format!(". The committed stream failed with: {failure}")
                }
                Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_)
                | Json::Object(_) => String::new(),
            };
            return Err(SessionCommitted {
                message: format!("{COMMITTED_MESSAGE}{suffix}"),
                status_code: 400,
            });
        }
        match &self.pending_session_update {
            Json::Text(update) => Ok(Some(update.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Ok(None)
            }
        }
    }

    pub fn commit(&mut self) {
        self.pending_session_update = Json::Null;
        self.session_committed = Json::Bool(true);
    }

    pub fn record_failure(&mut self, failure: String) {
        if matches!(self.session_committed, Json::Bool(true)) {
            self.committed_failure = Json::Text(failure);
        }
    }
}
