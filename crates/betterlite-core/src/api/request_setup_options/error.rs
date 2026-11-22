use betterlite_logging::callback_config::error::ParseError;
use betterlite_logging::logging_privacy::CallbackMetadataError;
use thiserror::Error;

use crate::api::request_metadata_safety::error::ErrorFacts;
use crate::api::request_principal_metadata::error::RequestPrincipalMetadataError;

const INTERNAL: ErrorFacts = ErrorFacts {
    status: 500,
    error_type: "internal_server_error",
    param: None,
    code: "500",
};

#[derive(Debug, Error)]
pub enum RequestSetupOptionsError {
    #[error(
        "Request has no session id. Send an `x-litellm-session-id` header or `metadata.session_id`. Required by `general_settings.missing_session_id: reject`."
    )]
    SessionRequired,
    #[error("the team's stored callback_settings are not valid")]
    CallbackSettings {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "the team's stored callback_settings name {name}, which is not a dynamic callback parameter"
    )]
    CallbackSettingsVariable { name: String },
    #[error("the team's default callback settings are not valid")]
    TeamCallbacks {
        #[source]
        source: RequestPrincipalMetadataError,
    },
}

impl RequestSetupOptionsError {
    #[must_use]
    pub const fn facts(&self) -> ErrorFacts {
        match self {
            Self::SessionRequired => ErrorFacts {
                status: 400,
                error_type: "bad_request_error",
                param: Some("session_id"),
                code: "400",
            },
            Self::CallbackSettings { .. }
            | Self::CallbackSettingsVariable { .. }
            | Self::TeamCallbacks { .. } => INTERNAL,
        }
    }
}

// why one key or team logging entry was skipped; LiteLLM warns and keeps going
#[derive(Debug, Error)]
pub enum CallbackEntryError {
    #[error("the logging entries are not a list")]
    NotAList,
    #[error("the entry is not a callback object")]
    Shape {
        #[source]
        source: ParseError,
    },
    #[error("Invalid callback variable: {name}")]
    Variable { name: String },
    #[error("Callback param '{name}' contains an 'os.environ/' reference")]
    EnvironmentReference { name: String },
    #[error("Invalid langfuse_environment")]
    LangfuseEnvironment {
        #[source]
        source: CallbackMetadataError,
    },
    #[error("Invalid langfuse_span_scope: must be one of ['full', 'llm_only']")]
    SpanScope,
}
