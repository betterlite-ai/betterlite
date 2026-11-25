use serde::Deserialize;
use thiserror::Error;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChunkKind {
    None,
    Json,
    Object,
    Broken,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QuotaKind {
    RateLimit,
    Internal,
}

// A scripted provider, stream or socket failure carries only the text the case scripts for it.
#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct ScriptedFailure {
    pub(crate) message: String,
}
