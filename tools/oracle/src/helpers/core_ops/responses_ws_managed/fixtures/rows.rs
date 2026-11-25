use serde::Deserialize;
use thiserror::Error;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::helpers::core_ops::responses_ws_managed) enum ChunkKind {
    None,
    Json,
    Object,
    Broken,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::helpers::core_ops::responses_ws_managed) enum QuotaKind {
    RateLimit,
    Internal,
}

// A scripted provider, stream or socket failure carries only the text the case scripts for it.
#[derive(Debug, Error)]
#[error("{message}")]
pub(in crate::helpers::core_ops::responses_ws_managed) struct ScriptedFailure {
    pub(in crate::helpers::core_ops::responses_ws_managed) message: String,
}
