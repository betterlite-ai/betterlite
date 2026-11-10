use std::io;

use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::credentials::CredentialsError;
use crate::error::ProviderError;

#[derive(Debug, Error)]
pub enum SdkImportError {
    #[error("cannot import WorkloadIdentityAuth from openai.auth")]
    WorkloadIdentityAuth,
    #[error("cannot import k8s_service_account_token_provider from openai.auth")]
    K8sTokenProvider,
}

#[derive(Debug, Error)]
pub enum TokenFileError {
    #[error("{0}")]
    Io(#[source] io::Error),
    #[error("The token file at {path} is empty.")]
    Empty { path: String },
    #[error("the token file at {path} is not valid UTF-8")]
    NotUtf8 { path: String },
    #[error("the token file at {path} is larger than {limit} bytes")]
    TooLarge { path: String, limit: usize },
}

#[derive(Debug, Error)]
pub enum WorkloadIdentityError {
    #[error("api_base is not a valid URL")]
    ApiBase(#[source] CredentialsError),
    #[error("{}", .provider.message)]
    SdkUnavailable {
        provider: Box<ProviderError>,
        #[source]
        cause: SdkImportError,
    },
    #[error("Failed to read the token file at {path}: {source}")]
    TokenFile {
        path: String,
        #[source]
        source: TokenFileError,
    },
    #[error("the token exchange URL is not valid")]
    ExchangeUrl(#[source] url::ParseError),
    #[error("the token exchange request does not encode")]
    Encode(#[source] serde_json::Error),
    #[error("the token exchange request failed")]
    Transport(#[source] TransportError),
    #[error("{message}")]
    OAuth {
        status: u16,
        error: Option<String>,
        message: String,
    },
    #[error("Token exchange succeeded but response body was empty")]
    EmptyBody,
    #[error("Token exchange response did not include a valid {field}")]
    InvalidField { field: &'static str },
    #[error("Token exchange failed with status {status}")]
    ExchangeStatus { status: u16 },
    #[error("Token is unusable after refresh completed")]
    TokenUnusable,
}
