use std::error::Error;

use http::header::InvalidHeaderValue;

use crate::batch::logger::SettingsError;

#[derive(Debug, thiserror::Error)]
pub enum CostManagementError {
    #[error("the Datadog cost management setting '{name}' is not supported")]
    UnknownSetting { name: String },
    #[error("the Datadog cost management setting '{name}' must be {expected}")]
    InvalidSetting {
        name: String,
        expected: &'static str,
    },
    #[error(transparent)]
    Batch(#[from] SettingsError),
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
}

#[derive(Debug, thiserror::Error)]
pub enum HeaderError {
    #[error("DD_API_KEY and DD_APP_KEY are required for the Datadog cost upload")]
    MissingCredentials,
    #[error("the {header} header could not be built")]
    Invalid {
        header: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError<TransportError>
where
    TransportError: Error + 'static,
{
    #[error("the Datadog cost upload URL '{url}' is not valid")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error(transparent)]
    Headers(#[from] HeaderError),
    #[error("the cost entries could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Datadog cost upload failed")]
    Transport(#[source] TransportError),
    #[error("Datadog answered the cost upload with status {status}")]
    Status { status: u16 },
}
