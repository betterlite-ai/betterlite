use std::io;

use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0}")]
    Usage(String),
    #[error("{0} is not set")]
    KeyNotSet(String),
    #[error("the environment has no home directory")]
    NoHome,
    #[error("the private JSON runtime could not start: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Save(#[from] PrivateJsonError),
}
