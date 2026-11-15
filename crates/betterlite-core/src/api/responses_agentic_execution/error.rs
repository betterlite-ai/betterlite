use crate::api::messages_fake_stream::MessagesFakeStreamError;
use crate::api::responses_hook_policy::error::{
    AgenticFollowupKwargsError, AgenticLoopSafetyError, AgenticLoopSettingsError,
};
use betterlite_context::agentic_callback::error::AgenticHookError;
use std::error::Error;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum AgenticExecutionError {
    #[error(transparent)]
    Settings(#[from] AgenticLoopSettingsError),
    #[error(transparent)]
    Safety(#[from] AgenticLoopSafetyError),
    #[error("optional_params.tools must be a list of tool objects")]
    InvalidTools {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Wrap(#[from] WrapError),
}
#[derive(Debug, Error)]
pub enum WrapError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    FakeStream(#[from] MessagesFakeStreamError),
}
#[derive(Debug, Error)]
pub enum PatchError {
    #[error("Agentic loop plan missing patched responses input")]
    MissingResponsesInput,
    #[error("Agentic loop plan missing patched messages")]
    MissingMessages,
    #[error("the agentic follow-up field `{field}` must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Depth(#[from] AgenticFollowupKwargsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
#[derive(Debug, Error)]
pub enum CallbackError<Source>
where
    Source: Error + 'static,
{
    #[error(transparent)]
    Hook(#[from] AgenticHookError),
    #[error(transparent)]
    Patch(#[from] PatchError),
    #[error(transparent)]
    Wrap(#[from] WrapError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Execution(Source),
}
