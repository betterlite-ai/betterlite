use std::io::Error as IoError;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_http::hub_token::{HubToken, HubTokenError, HubTokenResolver};
use hf_hub::{HFClient, HFClientBuilder, HFError};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::time::Instant;

mod classify;
mod download;

use classify::{not_available, read_file, retryable};
use download::{Bounds, RETRY_START_TIMEOUT, download_bounded};

pub(crate) struct Request<'request> {
    pub(crate) repository: &'request str,
    pub(crate) revision: &'request str,
    pub(crate) token: Option<&'request SecretString>,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Error)]
pub enum HubError {
    #[error("the hugging face token could not be resolved")]
    Token(#[source] HubTokenError),
    #[error("the hugging face hub client could not be created")]
    Client(#[source] HFError),
    #[error("the tokenizer repository {repository} was not found")]
    RepoNotFound {
        repository: String,
        #[source]
        source: HFError,
    },
    #[error("access to the tokenizer repository {repository} is gated")]
    Gated {
        repository: String,
        #[source]
        source: HFError,
    },
    #[error("the hugging face token was rejected")]
    Unauthorized {
        #[source]
        source: HFError,
    },
    #[error("revision {revision} of {repository} was not found")]
    RevisionNotFound {
        repository: String,
        revision: String,
        #[source]
        source: HFError,
    },
    #[error("the tokenizer file was not found in {repository}")]
    EntryNotFound {
        repository: String,
        #[source]
        source: HFError,
    },
    #[error("the tokenizer is not in the local cache for {repository}")]
    CacheMiss { repository: String, path: String },
    #[error("the tokenizer could not be downloaded from {repository}")]
    Download {
        repository: String,
        #[source]
        source: HFError,
    },
    #[error(
        "the tokenizer could not be downloaded from {repository} and is not in the local cache"
    )]
    NotAvailable {
        repository: String,
        #[source]
        source: Box<Self>,
    },
    #[error("downloading the tokenizer from {repository} timed out")]
    StartTimeout { repository: String },
    #[error("downloading the tokenizer from {repository} stalled for {seconds} seconds")]
    Stalled { repository: String, seconds: u64 },
    #[error("the cached tokenizer at {path} could not be read")]
    Read {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("the tokenizer timeout is too large to count in monotonic time")]
    TimeoutRange,
}

impl HubError {
    const fn is_terminal(&self) -> bool {
        match self {
            Self::RevisionNotFound { .. } | Self::EntryNotFound { .. } | Self::Stalled { .. } => {
                true
            }
            Self::Token(_)
            | Self::Client(_)
            | Self::RepoNotFound { .. }
            | Self::Gated { .. }
            | Self::Unauthorized { .. }
            | Self::CacheMiss { .. }
            | Self::Download { .. }
            | Self::NotAvailable { .. }
            | Self::StartTimeout { .. }
            | Self::Read { .. }
            | Self::TimeoutRange => false,
        }
    }

    pub(super) const fn keeps_identity(&self) -> bool {
        match self {
            Self::RepoNotFound { .. } | Self::Gated { .. } | Self::Unauthorized { .. } => true,
            Self::Token(_)
            | Self::Client(_)
            | Self::RevisionNotFound { .. }
            | Self::EntryNotFound { .. }
            | Self::CacheMiss { .. }
            | Self::Download { .. }
            | Self::NotAvailable { .. }
            | Self::StartTimeout { .. }
            | Self::Stalled { .. }
            | Self::Read { .. }
            | Self::TimeoutRange => false,
        }
    }
}

pub struct HubDownloader {
    pub(crate) settings: Arc<HubSettings>,
    pub(crate) tokens: Arc<HubTokenResolver>,
    pub(crate) clock: Arc<dyn Clock>,
    warned: AtomicBool,
}

impl HubDownloader {
    #[must_use]
    pub fn new(
        settings: Arc<HubSettings>,
        tokens: Arc<HubTokenResolver>,
        clock: Arc<dyn Clock>,
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
            let path = download_bounded(self, &request, true, bounds).await?;
            return read_file(&path).await;
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
        if first.is_terminal() {
            return Err(first);
        }
        match download_bounded(self, request, true, bounds).await {
            Ok(path) => read_file(&path).await,
            Err(_missing) => {
                if !retryable(&first) {
                    return Err(not_available(first, request.repository));
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
