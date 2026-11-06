use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_http::hub_token::{HubToken, HubTokenError, HubTokenResolver};
use hf_hub::{HFClient, HFClientBuilder, HFError, split_id};
use secrecy::{ExposeSecret as _, SecretString};
use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use tokio::time::Instant;
mod download;
use download::{Bounds, RETRY_START_TIMEOUT, download_bounded};
pub(crate) struct Request<'request> {
    pub(crate) repository: &'request str,
    pub(crate) revision: &'request str,
    pub(crate) token: Option<&'request SecretString>,
}
pub type Clock = dyn Fn() -> Instant + Send + Sync;
#[derive(Debug, Error)]
pub enum HubError {
    #[error("the hugging face token could not be resolved")]
    Token(#[source] HubTokenError),
    #[error("the hugging face hub client could not be created")]
    Client(#[source] HFError),
    #[error("the tokenizer repository {0} was not found")]
    RepoNotFound(String, #[source] HFError),
    #[error("access to the tokenizer repository {0} is gated")]
    Gated(String, #[source] HFError),
    #[error("the hugging face token was rejected")]
    Unauthorized(#[source] HFError),
    #[error("revision {1} of {0} was not found")]
    RevisionNotFound(String, String, #[source] HFError),
    #[error("the tokenizer file was not found in {0}")]
    EntryNotFound(String, #[source] HFError),
    #[error("the tokenizer is not in the local cache for {0}")]
    CacheMiss(String, String),
    #[error("the tokenizer could not be downloaded from {0}")]
    Download(String, #[source] HFError),
    #[error("the tokenizer could not be downloaded from {0} and is not in the local cache")]
    NotAvailable(String, #[source] Box<Self>),
    #[error("downloading the tokenizer from {0} timed out")]
    StartTimeout(String),
    #[error("downloading the tokenizer from {0} stalled for {1} seconds")]
    Stalled(String, u64),
    #[error("the cached tokenizer at {0} could not be read")]
    Read(PathBuf, #[source] IoError),
    #[error("the tokenizer timeout is too large to count in monotonic time")]
    TimeoutRange,
}
impl HubError {
    pub(super) const fn terminal(&self) -> bool {
        matches!(
            self,
            Self::RevisionNotFound(..) | Self::EntryNotFound(..) | Self::Stalled(..)
        )
    }
}
pub struct HubDownloader {
    pub(crate) settings: Arc<HubSettings>,
    pub(crate) tokens: Arc<HubTokenResolver>,
    pub(crate) clock: Arc<Clock>,
    warned: AtomicBool,
}
impl HubDownloader {
    #[must_use]
    pub fn new(
        settings: Arc<HubSettings>,
        tokens: Arc<HubTokenResolver>,
        clock: Arc<Clock>,
    ) -> Self {
        Self {
            settings,
            tokens,
            clock,
            warned: AtomicBool::new(false),
        }
    }
    pub async fn download(
        &self,
        repository: &str,
        revision: &str,
        explicit: Option<&SecretString>,
    ) -> Result<Vec<u8>, HubError> {
        let token = match explicit {
            Some(value) => Some(SecretString::from(value.expose_secret().to_owned())),
            None => self
                .tokens
                .resolve(&HubToken::Implicit)
                .await
                .map_err(HubError::Token)?,
        };
        let request = Request {
            repository,
            revision,
            token: token.as_ref(),
        };
        let bounds = Bounds {
            start: Duration::from_secs(self.settings.etag_timeout),
            idle: Duration::from_secs(self.settings.download_timeout),
        };
        if self.settings.offline == OfflineSetting::Offline {
            return read_file(&download_bounded(self, &request, true, bounds).await?).await;
        }
        match download_bounded(self, &request, false, bounds).await {
            Ok(path) => read_file(&path).await,
            Err(first) => self.after_failure(&request, first, bounds).await,
        }
    }
    async fn after_failure(
        &self,
        request: &Request<'_>,
        first: HubError,
        bounds: Bounds,
    ) -> Result<Vec<u8>, HubError> {
        if first.terminal() {
            return Err(first);
        }
        match download_bounded(self, request, true, bounds).await {
            Ok(path) => return read_file(&path).await,
            Err(_missing) if !retryable(&first) => {
                return Err(not_available(first, request.repository));
            }
            Err(_missing) => {}
        }
        let retry = Bounds {
            start: RETRY_START_TIMEOUT,
            idle: bounds.idle,
        };
        match download_bounded(self, request, false, retry).await {
            Ok(path) => read_file(&path).await,
            Err(second) => Err(not_available(second, request.repository)),
        }
    }
    pub(crate) fn client(&self, token: Option<&SecretString>) -> Result<HFClient, HubError> {
        let mut builder = HFClientBuilder::new()
            .endpoint(self.settings.endpoint.clone())
            .cache_dir(self.settings.hub_cache.clone());
        if let Some(value) = token {
            builder = builder.token(value.expose_secret().to_owned());
        }
        let client = builder.build().map_err(HubError::Client)?;
        if self.settings.transfer == TransferSetting::DeprecatedTransfer
            && !self.warned.swap(true, Ordering::SeqCst)
        {
            tracing::warn!(
                "The `HF_HUB_ENABLE_HF_TRANSFER` environment variable is deprecated as 'hf_transfer' is not used anymore. Please use `HF_XET_HIGH_PERFORMANCE` instead to enable high performance transfer with Xet. Visit https://huggingface.co/docs/huggingface_hub/package_reference/environment_variables#hfxethighperformance for more details."
            );
        }
        Ok(client)
    }
}
pub(super) async fn classify(
    client: &HFClient,
    error: HFError,
    repository: &str,
    revision: &str,
) -> HubError {
    if matches!(&error, HFError::RevisionNotFound { .. }) {
        return HubError::RevisionNotFound(repository.to_owned(), revision.to_owned(), error);
    }
    if let HFError::EntryNotFound { context, .. } = &error {
        let code = context
            .as_ref()
            .and_then(|found| found.error_code.as_deref());
        if code == Some("RepoNotFound") {
            return HubError::RepoNotFound(repository.to_owned(), error);
        }
        if code == Some("RevisionNotFound") {
            return HubError::RevisionNotFound(repository.to_owned(), revision.to_owned(), error);
        }
        if code == Some("EntryNotFound") {
            return HubError::EntryNotFound(repository.to_owned(), error);
        }
        return missing_entry(client, error, repository).await;
    }
    if matches!(&error, HFError::RepoNotFound { .. }) {
        return HubError::RepoNotFound(repository.to_owned(), error);
    }
    let gated = matches!(
        &error,
        HFError::AuthRequired { context } | HFError::Forbidden { context }
            if context.error_code.as_deref() == Some("GatedRepo")
    );
    if gated {
        return HubError::Gated(repository.to_owned(), error);
    }
    if matches!(&error, HFError::AuthRequired { .. }) {
        return HubError::Unauthorized(error);
    }
    if let HFError::LocalEntryNotFound { path } = error {
        return HubError::CacheMiss(repository.to_owned(), path);
    }
    HubError::Download(repository.to_owned(), error)
}
async fn missing_entry(client: &HFClient, error: HFError, repository: &str) -> HubError {
    let (owner, name) = split_id(repository);
    let exists = client
        .model(owner.to_owned(), name.to_owned())
        .exists()
        .send()
        .await;
    match exists {
        Ok(false) => HubError::RepoNotFound(repository.to_owned(), error),
        _ => HubError::EntryNotFound(repository.to_owned(), error),
    }
}
pub(super) fn retryable(error: &HubError) -> bool {
    let HubError::Download(_, source) = error else {
        return matches!(error, HubError::StartTimeout(_));
    };
    if let HFError::Request {
        source: transport, ..
    } = source
    {
        return transport.is_timeout() || transport.is_connect();
    }
    if let HFError::Http { context } = source {
        return matches!(context.status.as_u16(), 408 | 500 | 502 | 503 | 504);
    }
    matches!(source, HFError::RateLimited { .. })
}
pub(super) fn not_available(error: HubError, repository: &str) -> HubError {
    if matches!(
        &error,
        HubError::RepoNotFound(..) | HubError::Gated(..) | HubError::Unauthorized(..)
    ) {
        return error;
    }
    HubError::NotAvailable(repository.to_owned(), Box::new(error))
}
pub(super) async fn read_file(path: &Path) -> Result<Vec<u8>, HubError> {
    fs::read(path)
        .await
        .map_err(|source| HubError::Read(path.to_path_buf(), source))
}
