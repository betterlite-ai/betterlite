use std::io;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::UserUrlError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenApiError {
    #[error("the OpenAPI document is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("OpenAPI spec not found at {location}")]
    SpecNotFound { location: String },
    #[error("reading the OpenAPI spec at {location} failed")]
    SpecRead {
        location: String,
        #[source]
        source: io::Error,
    },
    #[error("fetching the OpenAPI spec at {location} failed")]
    SpecFetch {
        location: String,
        #[source]
        source: UserUrlError,
    },
    #[error("the OpenAPI spec at {location} answered with status {status}")]
    SpecStatus { location: String, status: u16 },
    #[error("reading the OpenAPI spec body at {location} failed")]
    SpecBody {
        location: String,
        #[source]
        source: TransportError,
    },
}
