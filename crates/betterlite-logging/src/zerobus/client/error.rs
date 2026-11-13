use http::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;

use crate::zerobus::client::types::ZerobusIngestFailure;

#[derive(Debug, Error)]
pub enum ZerobusClientError {
    #[error("the {what} URL is not valid")]
    Url {
        what: &'static str,
        #[source]
        source: ParseError,
    },
    #[error("the service principal credentials cannot be carried in an Authorization header")]
    CredentialHeader(#[source] InvalidHeaderValue),
    #[error("the minted access token cannot be carried in an Authorization header")]
    TokenHeader(#[source] InvalidHeaderValue),
    #[error("the table name {table_name:?} is not catalog.schema.table")]
    TableName { table_name: String },
    #[error("the authorization details cannot be written as JSON")]
    AuthorizationDetails(#[source] serde_json::Error),
    #[error("the rows cannot be written as JSON")]
    Rows(#[source] serde_json::Error),
}

impl From<ZerobusClientError> for ZerobusIngestFailure {
    fn from(error: ZerobusClientError) -> Self {
        Self {
            detail: error.to_string(),
            retryable: false,
        }
    }
}
