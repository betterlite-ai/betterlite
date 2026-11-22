use betterlite_a2a::send::error::SendCallError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("the A2A agent URL is not a valid URL")]
    AgentUrl(#[source] url::ParseError),
    #[error("the extra header name {name:?} is not a valid header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the extra header {name} holds a character a header cannot carry")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the A2A agent could not be reached")]
    Send(#[source] TransportError),
    #[error("the A2A agent's error answer could not be read")]
    ErrorBody(#[source] TransportError),
    #[error("the A2A agent's event stream broke off")]
    Body(#[source] HttpError),
    #[error("the A2A agent sent nothing for {seconds} seconds")]
    ReadTimeout { seconds: u64 },
    #[error("the A2A agent sent a line longer than {limit} bytes")]
    LineTooLong { limit: usize },
    // the UpstreamAgentStream impl reports a failed A2A send call through this; boxed since the call error is large
    #[error(transparent)]
    Agent(Box<SendCallError>),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StreamFailure {
    #[error("{detail}")]
    Rejected { status: u16, detail: String },
    #[error("{message}")]
    Failed { message: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct StreamAbort {
    pub message: String,
}
