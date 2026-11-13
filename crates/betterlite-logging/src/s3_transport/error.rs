use aws_sigv4::http_request::SigningError;
use aws_sigv4::sign::v4::signing_params::BuildError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};

use crate::s3_transport::signing::CredentialError;

#[derive(Debug, thiserror::Error)]
pub enum S3TransportError {
    #[error("the S3 object URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("no S3 bucket is configured")]
    MissingBucket,
    #[error("the AWS region {region} is not valid")]
    Region { region: String },
    #[error("the S3 request header name {name} is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the S3 request header value of {name} is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the S3 request could not be sent")]
    Send(#[from] TransportError),
    #[error("the SigV4 signing parameters are invalid")]
    Parameters(#[from] BuildError),
    #[error("the S3 request could not be signed")]
    Signing(#[from] SigningError),
    #[error("the AWS credentials for S3 could not be resolved")]
    Credentials(#[from] CredentialError),
    #[error("the S3 logging payload could not be serialized")]
    Payload(#[from] serde_json::Error),
}
