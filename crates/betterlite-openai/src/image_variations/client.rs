use std::time::Duration;

use betterlite_config::sections::openai_credentials::DEFAULT_API_BASE;
use secrecy::SecretString;
use url::Url;

use super::error::ImageVariationSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

impl ClientKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
        }
    }
}

// LiteLLM builds every variation client on this shared session; betterlite's pooled client plays that part
#[derive(Debug, Clone)]
pub struct SharedSession;

// a client the caller already built, or one selected from the call settings; image_variations hands a supplied one back
#[derive(Debug, Clone)]
pub struct ImageVariationClient {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub base_url: String,
    pub timeout: Option<Duration>,
    pub max_retries: i64,
    pub organization: Option<String>,
    pub shared_session: bool,
}

pub struct ClientSettings<'settings> {
    pub kind: ClientKind,
    pub api_key: Option<&'settings SecretString>,
    pub api_base: Option<&'settings str>,
    pub organization: Option<&'settings str>,
    pub timeout: Option<Duration>,
    pub max_retries: i64,
    pub supplied: Option<&'settings ImageVariationClient>,
    pub shared_session: bool,
}

// get_sync_client and get_async_client: a supplied client keeps every setting it was built with
pub fn select_client(
    settings: &ClientSettings<'_>,
) -> Result<ImageVariationClient, ImageVariationSource> {
    let Some(supplied) = settings.supplied else {
        let api_key = settings
            .api_key
            .cloned()
            .ok_or(ImageVariationSource::MissingApiKey)?;
        return Ok(ImageVariationClient {
            kind: settings.kind,
            api_key: Some(api_key),
            base_url: normalize_base(settings.api_base.unwrap_or(DEFAULT_API_BASE))?,
            timeout: settings.timeout,
            max_retries: settings.max_retries,
            organization: settings.organization.map(str::to_owned),
            shared_session: settings.shared_session,
        });
    };
    if supplied.kind != settings.kind {
        return Err(ImageVariationSource::ClientKind {
            supplied: supplied.kind.label(),
            needed: settings.kind.label(),
        });
    }
    Ok(ImageVariationClient {
        shared_session: false,
        ..supplied.clone()
    })
}

// the OpenAI SDK's base_url always ends in a slash, so /images/variations is appended to it
pub fn normalize_base(api_base: &str) -> Result<String, ImageVariationSource> {
    let mut url = Url::parse(api_base).map_err(ImageVariationSource::Url)?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.to_string())
}
