use async_trait::async_trait;
use betterlite_http::error::HttpError;
use thiserror::Error;

// urllib's HTTPRedirectHandler follows at most this many redirects, then raises
pub const MAX_REDIRECTS: usize = 10;
pub const FETCH_TIMEOUT_SECONDS: u64 = 10;

#[derive(Debug, Error)]
pub enum MetadataFetchError {
    #[error("the IdP metadata URL is not valid")]
    InvalidUrl,
    #[error("the IdP metadata request failed")]
    Request(#[source] HttpError),
    #[error("the IdP metadata endpoint answered with status {status}")]
    Status { status: u16 },
    #[error("the IdP metadata endpoint redirected more than {limit} times")]
    TooManyRedirects { limit: usize },
}

// OneLogin_Saml2_IdPMetadataParser.get_metadata: fetch the published XML, certificate checks aside
#[async_trait]
pub trait MetadataFetcher: Send + Sync {
    async fn fetch(
        &self,
        url: &str,
        verify_certificate: bool,
    ) -> Result<Vec<u8>, MetadataFetchError>;
}
