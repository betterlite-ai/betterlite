use http::header::{InvalidHeaderName, InvalidHeaderValue, ToStrError};
use http::method::InvalidMethod;
use http::uri::InvalidUri;
use thiserror::Error;
use tokio::time::error::Elapsed;
use url::ParseError;

use betterlite_http::error::HttpError;
use betterlite_http::mcp_sigv4::error::McpSigV4Error;
use betterlite_http::tls::TlsError;

use crate::client::cleanup::http_client::ResponseHead;
use crate::credentials::CredentialError;

#[derive(Debug, Error)]
pub enum McpHttpError {
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error(transparent)]
    Client(#[from] HttpError),
    #[error("the MCP HTTP request timed out")]
    Timeout(#[source] HttpError),
    #[error("the MCP HTTP server could not be reached")]
    Connect(#[source] HttpError),
    #[error("the MCP HTTP request failed")]
    Request(#[source] HttpError),
    #[error("the MCP HTTP request was not answered in time")]
    HeadTimeout(#[source] Elapsed),
    #[error("the MCP HTTP response body stopped arriving in time")]
    BodyTimeout(#[source] Elapsed),
    #[error("the MCP HTTP response body could not be read")]
    Body(#[source] HttpError),
    #[error("the MCP HTTP response body is larger than {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    SigV4(#[from] McpSigV4Error),
    #[error("the credential changed the request method to an unsupported one")]
    CredentialMethod(#[source] InvalidMethod),
    #[error("the credential changed the request URL to an invalid one")]
    CredentialUrl(#[source] ParseError),
    #[error("the {name} header name is not valid for an HTTP request")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not valid for an HTTP request")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the {name} header value is not visible text")]
    HeaderText {
        name: String,
        #[source]
        source: ToStrError,
    },
    #[error("the {name} header is repeated, and a credential carries one value per name")]
    RepeatedHeader { name: String },
    #[error("the request URL cannot be sent as an HTTP request target")]
    Uri(#[source] InvalidUri),
    #[error("the MCP server answered HTTP {}", .0.status)]
    Status(Box<ResponseHead>),
}
