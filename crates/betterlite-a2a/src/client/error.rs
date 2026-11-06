use std::time::Duration;

use betterlite_core::a2a::exception_mapping::A2AException;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::client::convert::ConvertError;
use crate::errors::A2AError;
use crate::errors::retries::RetryCounts;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("the connection to the agent at {url} failed")]
    Connection {
        url: String,
        #[source]
        source: TransportError,
    },
    #[error("the agent at {url} did not answer within {timeout:?}")]
    Timeout {
        url: String,
        timeout: Duration,
        #[source]
        source: Option<TransportError>,
    },
    #[error("the request to the agent at {url} could not be sent")]
    Request {
        url: String,
        #[source]
        source: TransportError,
    },
    #[error("the agent at {url} answered HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("the agent at {url} returned JSON-RPC error {code}: {message}")]
    JsonRpc {
        url: String,
        code: i64,
        message: String,
    },
    #[error("the answer from the agent at {url} could not be read")]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the request for the agent at {url} could not be built")]
    Encode {
        url: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer from the agent at {url} is not a JSON-RPC A2A response")]
    Protocol { url: String },
    #[error("the event stream from the agent at {url} is larger than the buffer allows")]
    EventTooLarge { url: String },
    #[error("{message}")]
    Final { message: &'static str },
    #[error("an extra A2A request header name is not valid")]
    InvalidHeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("an extra A2A request header carries a value that is not valid")]
    InvalidHeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error(transparent)]
    Convert(#[from] ConvertError),
    #[error("the agent card endpoint is not a usable URL")]
    Endpoint {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the agent card has no endpoint URL")]
    MissingEndpoint,
}

#[must_use]
pub fn exception_to_a2a(exception: A2AException) -> A2AError {
    match exception {
        A2AException::LocalhostUrl(error) => A2AError::LocalhostUrl {
            localhost_url: error.localhost_url,
            base_url: error.base_url,
            model: error.model.clone(),
            original_error: Some(Box::new(A2AError::Base {
                message: error.original_error,
                model: error.model,
                debug_info: None,
                retries: RetryCounts::default(),
            })),
        },
        A2AException::AgentCard { message, url, .. } => A2AError::AgentCard {
            message,
            url,
            status_code: 404,
        },
        A2AException::Connection {
            message,
            url,
            model,
        } => A2AError::Connection {
            message,
            url,
            model,
            debug_info: None,
            retries: RetryCounts::default(),
        },
        A2AException::Other { message, model } => A2AError::Base {
            message,
            model,
            debug_info: None,
            retries: RetryCounts::default(),
        },
    }
}

#[must_use]
pub fn transport_error(url: &str, deadline: Duration, source: TransportError) -> ClientError {
    if matches!(source, TransportError::Deadline(_)) {
        return ClientError::Timeout {
            url: url.to_owned(),
            timeout: deadline,
            source: Some(source),
        };
    }
    let connection = matches!(
        source,
        TransportError::Connect(_)
            | TransportError::ConnectTimeout(_)
            | TransportError::Request(_)
            | TransportError::Body(_)
    );
    if connection {
        return ClientError::Connection {
            url: url.to_owned(),
            source,
        };
    }
    ClientError::Request {
        url: url.to_owned(),
        source,
    }
}
