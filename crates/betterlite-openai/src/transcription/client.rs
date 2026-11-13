use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::error::TranscriptionError;

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

// LiteLLM builds the async transport on this shared aiohttp session; betterlite's pooled client plays that part.
#[derive(Debug, Clone)]
pub struct SharedSession;

#[derive(Debug, Clone)]
pub struct TranscriptionClient {
    pub kind: ClientKind,
    pub api_key: SecretString,
    pub base_url: Url,
    pub timeout: Duration,
    pub max_retries: u32,
    pub shared_session: Option<SharedSession>,
}

// The arguments OpenAIAudioTranscription.audio_transcriptions hands _get_openai_client.
#[derive(Debug, Clone)]
pub struct TranscriptionConnection {
    pub api_key: Option<SecretString>,
    pub environment_api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
    pub shared_session: Option<SharedSession>,
}

// The OpenAI SDK keeps a base URL whose path ends in a slash, so audio/transcriptions appends instead of replacing.
pub fn select_client(
    connection: &TranscriptionConnection,
    kind: ClientKind,
) -> Result<TranscriptionClient, TranscriptionError> {
    // the explicit key wins; there is no module-level key fallback
    let key = connection
        .api_key
        .as_ref()
        .or(connection.environment_api_key.as_ref())
        .ok_or(TranscriptionError::MissingApiKey)?;
    let api_base = connection.api_base.as_deref().unwrap_or(DEFAULT_API_BASE);
    let mut base_url =
        Url::parse(api_base).map_err(|source| TranscriptionError::BaseUrl { source })?;
    if !base_url.path().ends_with('/') {
        let path = format!("{}/", base_url.path());
        base_url.set_path(&path);
    }
    Ok(TranscriptionClient {
        kind,
        api_key: SecretString::from(key.expose_secret()),
        base_url,
        timeout: connection.timeout,
        max_retries: connection.max_retries,
        shared_session: connection
            .shared_session
            .clone()
            .filter(|_session| kind == ClientKind::Async),
    })
}
