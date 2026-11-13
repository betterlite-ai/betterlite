use async_trait::async_trait;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedUpload {
    pub url: String,
    pub host_header: Option<String>,
}

#[async_trait]
pub trait UploadUrlCheck: Send + Sync {
    async fn check(&self, url: &str) -> Result<ValidatedUpload, UserUrlError>;
}

#[async_trait]
impl<Resolver: Resolve> UploadUrlCheck for UrlValidator<Resolver> {
    async fn check(&self, url: &str) -> Result<ValidatedUpload, UserUrlError> {
        let validated = self.validate_url(url).await?;
        Ok(ValidatedUpload {
            url: validated.url,
            host_header: (!validated.host_header.is_empty()).then_some(validated.host_header),
        })
    }
}
