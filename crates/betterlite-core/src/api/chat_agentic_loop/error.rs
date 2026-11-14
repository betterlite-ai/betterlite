use std::error::Error;

use betterlite_context::agentic_callback::AgenticHookError;
use betterlite_context::agentic_followup_kwargs::AgenticFollowupKwargsError;
use thiserror::Error;

use super::safety::SafetyError;
use super::settings::SettingsError;

#[derive(Debug, Error)]
pub enum AgenticLoopError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Safety(#[from] SafetyError),
    #[error("optional_params.tools must be a list")]
    InvalidTools {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

// the plan execution's errors: the guard in entry.rs catches them and moves to the next callback
#[derive(Debug, Error)]
pub enum ChatCallError<Source>
where
    Source: Error + 'static,
{
    #[error("Agentic loop plan missing patched messages")]
    MissingMessages,
    #[error(transparent)]
    Hook(#[from] AgenticHookError),
    #[error(transparent)]
    Depth(#[from] AgenticFollowupKwargsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Completion(Source),
}
