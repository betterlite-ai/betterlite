use thiserror::Error;

use crate::api::claude_plugin_policy::{
    ClaudePluginPolicyError, PluginErrorDetail, PluginErrorFields,
};

#[derive(Debug, Error)]
pub enum PluginManifestError {
    #[error("the stored plugin manifest is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Error)]
pub enum ClaudePluginReadFailure {
    #[error(transparent)]
    Policy(#[from] ClaudePluginPolicyError),
    #[error("{message}")]
    Manifest { message: String },
}

impl ClaudePluginReadFailure {
    #[must_use]
    pub fn fields(&self) -> PluginErrorFields {
        match self {
            Self::Policy(error) => error.fields(),
            Self::Manifest { message } => PluginErrorFields {
                status_code: 500,
                detail: PluginErrorDetail {
                    error: message.clone(),
                },
            },
        }
    }
}

// get_plugin's 403, also raised by the handler before it parses the stored manifest
#[must_use]
pub fn plugin_not_granted_error(plugin_name: &str) -> ClaudePluginPolicyError {
    ClaudePluginPolicyError::Response {
        status_code: 403,
        message: format!("Plugin '{plugin_name}' is not granted to this key"),
    }
}
