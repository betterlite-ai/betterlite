use std::time::Duration;

use secrecy::SecretString;
use url::Url;

use crate::speech::error::SpeechError;

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

// the OpenAI SDK's default request timeout, kept when the call names none
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

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

// LiteLLM builds the async transport on this shared aiohttp session; betterlite's pooled client plays that part.
#[derive(Debug, Clone)]
pub struct SharedSession;

// A client the caller already built: _get_openai_client keeps its settings and only overwrites max_retries.
#[derive(Debug, Clone)]
pub struct SuppliedClient {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub base_url: Url,
    pub timeout: Duration,
    pub max_retries: u32,
    pub organization: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpeechClient {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub base_url: Url,
    pub timeout: Duration,
    pub max_retries: u32,
    pub organization: Option<String>,
    pub shared_session: Option<SharedSession>,
}

// The arguments OpenAIChatCompletion.audio_speech hands _get_openai_client; organization and project stay out of it.
#[derive(Debug, Clone)]
pub struct SpeechConnection {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub client: Option<SuppliedClient>,
    pub shared_session: Option<SharedSession>,
}

// The OpenAI SDK's base_url: a URL whose path ends in a slash, so audio/speech appends instead of replacing.
pub fn normalize_base(api_base: &str) -> Result<Url, SpeechError> {
    let mut url = Url::parse(api_base).map_err(|source| SpeechError::BaseUrl { source })?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

pub fn select_client(
    connection: &SpeechConnection,
    kind: ClientKind,
) -> Result<SpeechClient, SpeechError> {
    let Some(supplied) = &connection.client else {
        let api_key = connection
            .api_key
            .clone()
            .ok_or(SpeechError::MissingApiKey)?;
        let base_url = normalize_base(connection.api_base.as_deref().unwrap_or(DEFAULT_API_BASE))?;
        let shared_session = if kind == ClientKind::Async {
            connection.shared_session.clone()
        } else {
            None
        };
        return Ok(SpeechClient {
            kind,
            api_key: Some(api_key),
            base_url,
            timeout: connection.timeout,
            max_retries: connection.max_retries,
            organization: None,
            shared_session,
        });
    };
    if supplied.kind != kind {
        return Err(SpeechError::ClientKind {
            supplied: supplied.kind.label(),
            needed: kind.label(),
        });
    }
    Ok(SpeechClient {
        kind,
        api_key: supplied.api_key.clone(),
        base_url: supplied.base_url.clone(),
        timeout: supplied.timeout,
        max_retries: connection.max_retries,
        organization: supplied.organization.clone(),
        shared_session: None,
    })
}
