use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::api::soniox::prepare::error::ReplyError;
use crate::api::soniox::records::error::SonioxException;

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error(transparent)]
    Provider(#[from] SonioxException),
    #[error(transparent)]
    Reply(#[from] ReplyError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the Soniox endpoint {url} is not a URL this request can be sent to")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name cannot be sent")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value cannot be sent")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the authenticated headers carry no {name} header")]
    MissingHeader { name: &'static str },
    #[error("the {name} reply header is not text")]
    ReplyHeader { name: String },
    #[error("the polling interval of {seconds} seconds cannot be waited for")]
    Interval { seconds: Decimal },
}
