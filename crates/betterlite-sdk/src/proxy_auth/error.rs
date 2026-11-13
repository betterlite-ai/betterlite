use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AzureCredentialError<E> {
    #[error(
        "azure-identity is required for AzureADCredential. Install it with: pip install azure-identity"
    )]
    ImportError,
    #[error("the Azure credential could not produce a token")]
    Credential(#[source] E),
}

#[derive(Debug, Error)]
pub enum OAuth2Error {
    #[error("the token request could not be completed")]
    Transport(#[source] TransportError),
    #[error("the token endpoint answered with status {status}")]
    Status { status: u16 },
    #[error("the token endpoint answered with something that is not a token response")]
    Body(#[source] serde_json::Error),
    #[error("the token lifetime runs past the largest timestamp")]
    ExpiryOverflow,
}

#[derive(Debug, Error)]
pub enum ProxyAuthError<E> {
    #[error("the credential could not produce a token")]
    Credential(#[source] E),
    #[error("the token cannot be carried in an Authorization header")]
    InvalidHeader(#[source] InvalidHeaderValue),
}
