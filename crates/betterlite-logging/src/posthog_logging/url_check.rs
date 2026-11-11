use async_trait::async_trait;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};

#[async_trait]
pub trait UrlCheck: Send + Sync {
    async fn check(&self, url: &str) -> Result<(), UserUrlError>;
}

#[async_trait]
impl<Resolver: Resolve> UrlCheck for UrlValidator<Resolver> {
    async fn check(&self, url: &str) -> Result<(), UserUrlError> {
        self.validate_url(url).await.map(|_| ())
    }
}
