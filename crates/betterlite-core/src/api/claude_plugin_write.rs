use regex::Regex;
use serde::Serialize;
use thiserror::Error;

use crate::api::claude_plugin_policy::ClaudePluginPolicyError;
use crate::api::claude_plugin_policy::source::{
    PATTERN_INVALID, PluginSource, PluginSpec, validate_plugin_source,
};

mod tests;

pub const PLUGIN_NAME_PATTERN: &str = "^[a-z0-9-]+$";
pub const PLUGIN_NAME_INVALID: &str =
    "Plugin name must be kebab-case (lowercase letters, numbers, hyphens)";
pub const REGISTRATION_FAILED: &str = "Registration failed";
pub const UPDATE_FAILED: &str = "Update failed";
pub const SUCCESS: &str = "success";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudePluginWriteAction {
    Create,
    Update,
    Enable,
    Disable,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudePluginWriteInput {
    pub action: ClaudePluginWriteAction,
    pub plugin_name: String,
    pub spec: Option<PluginSpec>,
}

#[derive(Debug, Error)]
pub enum ClaudePluginWriteFailure {
    #[error(transparent)]
    Policy(#[from] ClaudePluginPolicyError),
    #[error("the write input carries no plugin specification")]
    MissingSpec,
}

impl ClaudePluginWriteInput {
    pub fn validate(&self) -> Result<(), ClaudePluginWriteFailure> {
        match self.action {
            ClaudePluginWriteAction::Create => {
                if !plugin_name_is_valid(&self.plugin_name)? {
                    return Err(plugin_name_error());
                }
                self.validate_source()
            }
            ClaudePluginWriteAction::Update => self.validate_source(),
            ClaudePluginWriteAction::Enable
            | ClaudePluginWriteAction::Disable
            | ClaudePluginWriteAction::Delete => Ok(()),
        }
    }

    pub fn fields(&self) -> Result<PluginFields, ClaudePluginWriteFailure> {
        self.spec().map(|spec| PluginFields {
            version: spec.version.clone(),
            description: spec.description.clone(),
            source: spec.source.clone(),
        })
    }

    fn spec(&self) -> Result<&PluginSpec, ClaudePluginWriteFailure> {
        self.spec
            .as_ref()
            .ok_or(ClaudePluginWriteFailure::MissingSpec)
    }

    fn validate_source(&self) -> Result<(), ClaudePluginWriteFailure> {
        validate_plugin_source(&self.spec()?.source)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginFields {
    pub version: Option<String>,
    pub description: Option<String>,
    pub source: PluginSource,
}

fn plugin_name_error() -> ClaudePluginWriteFailure {
    ClaudePluginWriteFailure::Policy(ClaudePluginPolicyError::Response {
        status_code: 400,
        message: PLUGIN_NAME_INVALID.to_owned(),
    })
}

fn plugin_name_is_valid(name: &str) -> Result<bool, ClaudePluginPolicyError> {
    let pattern =
        Regex::new(PLUGIN_NAME_PATTERN).map_err(|source| ClaudePluginPolicyError::Pattern {
            message: PATTERN_INVALID,
            source,
        })?;
    Ok(pattern.is_match(name))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPlugin {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistrationAction {
    Created,
    Updated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub source: PluginSource,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginRegistration {
    pub status: &'static str,
    pub action: RegistrationAction,
    pub plugin: PluginView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginMessage {
    pub status: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ClaudePluginWriteResult {
    Registration(Box<PluginRegistration>),
    Message(PluginMessage),
}
