use std::error::Error;

use betterlite_context::agentic_callback::error::AgenticHookError;
use thiserror::Error;

use crate::api::messages_fake_stream::MessagesFakeStreamError;
use crate::api::responses_hook_policy::error::{
    AgenticFollowupKwargsError, AgenticLoopSafetyError, AgenticLoopSettingsError,
};

// what leaves the shared dispatcher: the rails and the settings, and a failed synthetic wrap
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

// a plan the callback could not turn into a follow-up call
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

// the per-callback guard in dispatch.rs logs these and moves on to the next callback
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
