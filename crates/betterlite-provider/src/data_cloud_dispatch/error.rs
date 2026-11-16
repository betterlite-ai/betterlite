use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

use crate::api::file_contracts::error::BaseLlmException;
use crate::databricks_auth::error::DatabricksError;
use crate::oci_common::OciError;
use crate::operation::OperationError;
use crate::snowflake::SnowflakeError;
use crate::snowflake::embedding::EmbeddingError as SnowflakeEmbeddingError;

use super::handler::HandlerError;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Databricks(#[from] DatabricksError),
    #[error("the {name} setting could not be read: {source}")]
    Environment {
        name: &'static str,
        #[source]
        source: SecretLookupError,
    },
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
    #[error("the request body could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider answer could not be read as JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {url} endpoint is not a valid URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the request headers cannot be carried")]
    Headers {
        #[source]
        source: http::Error,
    },
    #[error("the provider answered with status {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error(transparent)]
    Oci(#[from] OciError),
    #[error(transparent)]
    Snowflake(#[from] SnowflakeError),
    #[error(transparent)]
    SnowflakeEmbedding(#[from] SnowflakeEmbeddingError),
    #[error(transparent)]
    SnowflakeException(#[from] BaseLlmException),
    #[error("the OCI stream failed: {source}")]
    Stream {
        #[source]
        source: betterlite_http::error::HttpError,
    },
    #[error(transparent)]
    Transport(#[from] OperationError),
    #[error(transparent)]
    Handler(#[from] HandlerError),
}
