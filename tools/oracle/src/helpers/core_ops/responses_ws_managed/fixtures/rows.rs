use serde::Deserialize;
use thiserror::Error;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ChunkKind {
    None,
    Json,
    Object,
    Broken,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum QuotaKind {
    RateLimit,
    Internal,
}

// A scripted provider, stream or socket failure carries only the text the case scripts for it.
#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct ScriptedFailure {
    pub(super) message: String,
}
