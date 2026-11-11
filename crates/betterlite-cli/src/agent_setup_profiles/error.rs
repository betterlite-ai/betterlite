use std::io;
use std::path::PathBuf;

use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

use super::Target;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::codex_settings::error::CodexSettingsError;
use crate::config::error::ConfigError;

#[derive(Debug, Error)]
pub enum AgentSetupProfilesError {
    #[error("Gateway must be normalized")]
    BaseUrlNotNormalized,
    #[error("Invalid gateway URL: {0}")]
    BaseUrlInvalid(#[source] ConfigError),
    #[error("Invalid virtual key")]
    ApiKeyInvalid,
    #[error("Invalid model choice")]
    ModelInvalid,
    #[error("the environment has no home directory for the {0} setup")]
    Path(Target),
    #[error("could not resolve the {0} settings path: {1}")]
    ClaudeSettings(Target, #[source] ClaudeSettingsError),
    #[error("could not resolve the {0} settings path: {1}")]
    CodexSettings(Target, #[source] CodexSettingsError),
    #[error("could not resolve the {0} settings path {1}: {2}")]
    ResolveSettings(Target, PathBuf, #[source] io::Error),
    #[error("Could not read saved {0} setup at {1}; no settings were changed")]
    Read(Target, PathBuf, #[source] io::Error),
    #[error(
        "Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed"
    )]
    InvalidJson(Target, PathBuf, #[source] serde_json::Error),
    #[error(
        "Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed"
    )]
    InvalidValues(Target, PathBuf, #[source] Box<Self>),
    #[error(
        "Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed"
    )]
    InvalidPath(Target, PathBuf, #[source] io::Error),
    #[error(
        "Saved {0} setup at {1} is invalid or unsupported. Run `lite unconfigure {0} --forget` to discard it; no settings were changed"
    )]
    Invalid(Target, PathBuf),
    #[error("Could not lock agent setup; retry when other configure commands finish")]
    LockPrepare(PathBuf, #[source] PrivateJsonError),
    #[error("Could not lock agent setup; retry when other configure commands finish")]
    LockIo(PathBuf, #[source] io::Error),
    #[error("Could not lock agent setup; retry when other configure commands finish")]
    Locked,
    #[error("Could not save {0} setup; no {0} settings were changed")]
    Save(Target, #[source] PrivateJsonError),
    #[error("Could not remove saved {0} setup at {1}")]
    Forget(Target, PathBuf, #[source] io::Error),
}
