use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TokenRecordError {
    #[error("cannot encode the credential JSON: {source}")]
    Encode {
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot prepare the private token directory: {source}")]
    Directory {
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot stage the token record: {source}")]
    Stage {
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot commit the staged token record: {source}")]
    Commit {
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot write the token record: {source}")]
    Write {
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot overwrite the token record secret: {source}")]
    Overwrite {
        #[source]
        source: PrivateJsonError,
    },
}
