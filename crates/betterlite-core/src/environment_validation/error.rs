use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum EnvironmentValidationError {
    #[error("BadRequestError (status 400) for model {model}, with an empty message and provider")]
    BadRequest { model: String },
    #[error("cannot read the config file {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the config file {path} is not a JSON object: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("the config file {path} is larger than {limit} bytes")]
    ConfigTooLarge { path: PathBuf, limit: u64 },
}
