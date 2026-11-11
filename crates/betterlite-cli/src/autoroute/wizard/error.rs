use std::io;
use std::path::PathBuf;

use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

use crate::autoroute::config::error::ConfigError;
use crate::settings_backup::error::BackupError;

#[derive(Debug, Error)]
pub enum ModelDiscoveryError {
    #[error("the /v1/models response was not a JSON list of model objects")]
    NotAList(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum WizardError {
    #[error(transparent)]
    Discovery(#[from] ModelDiscoveryError),
    #[error("Your key has no chat-capable models available on this proxy.")]
    NoChatModels,
    #[error("`lite autoroute configure` requires an interactive terminal.")]
    NotInteractive,
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("the prompt could not run")]
    Prompt(#[from] inquire::InquireError),
    #[error("failed to write to the terminal")]
    Output(#[source] io::Error),
    #[error("the file runtime could not start")]
    Runtime(#[source] io::Error),
    #[error("cannot prepare the autorouter directory")]
    Directory(#[source] PrivateJsonError),
    #[error("cannot create {path}")]
    Create {
        path: PathBuf,
        #[source]
        source: Box<BackupError>,
    },
    #[error("cannot write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot serialize the autoroute configuration")]
    Serialize(#[source] serde_saphyr::SerializeError),
    #[error("the medium tier was not assigned a model")]
    NoDefaultModel,
}
