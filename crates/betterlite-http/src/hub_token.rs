use std::path::Path;
use std::path::PathBuf;
use std::str::{self, Utf8Error};
use std::{io, sync::Arc};

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::fs;
use tokio::sync::Mutex;

use betterlite_config::sections::huggingface_hub::Settings;
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;

use crate::hub_oauth::clock::{ClockRangeError, HubClock};
use crate::hub_oauth::error::{OidcError, StoredTokensError};
use crate::hub_oauth::oidc::OidcExchange;
use crate::hub_oauth::refresh::{LOCK_TIMEOUT, OauthRefresh, State};
use crate::proxy::ProxiedClient;

#[derive(Debug)]
pub enum HubToken {
    Explicit(SecretString),
    Implicit,
}

#[derive(Debug, Error)]
pub enum HubTokenError {
    #[error("failed to read Hugging Face token at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Hugging Face token at {path} is not UTF-8: {source}")]
    Utf8 {
        path: PathBuf,
        #[source]
        source: Utf8Error,
    },
    #[error(transparent)]
    Oidc(#[from] OidcError),
    #[error(transparent)]
    StoredTokens(#[from] StoredTokensError),
    #[error(transparent)]
    Clock(#[from] ClockRangeError),
}

pub struct HubTokenResolver {
    pub settings: Arc<Settings>,
    pub file_reader: Arc<dyn HubTokenFileReader>,
    pub oidc: OidcExchange,
    pub oauth: OauthRefresh,
}

#[async_trait::async_trait]
pub trait HubTokenFileReader: Send + Sync {
    async fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
}

#[derive(Debug, Default)]
pub struct TokioHubTokenFileReader;

#[async_trait::async_trait]
impl HubTokenFileReader for TokioHubTokenFileReader {
    async fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path).await
    }
}

mod tests;

// huggingface_hub's _get_token_from_file: a missing or blank file is no token
pub async fn read_token_file(
    reader: &dyn HubTokenFileReader,
    path: &Path,
) -> Result<Option<SecretString>, HubTokenError> {
    match reader.read(path).await {
        Ok(bytes) => {
            let text = str::from_utf8(&bytes).map_err(|source| HubTokenError::Utf8 {
                path: path.to_path_buf(),
                source,
            })?;
            let cleaned = text.replace(['\r', '\n'], "").trim().to_owned();
            Ok((!cleaned.is_empty()).then(|| SecretString::from(cleaned)))
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HubTokenError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

impl HubTokenResolver {
    #[must_use]
    pub fn new(
        settings: Settings,
        oidc: OidcSettings,
        client: Arc<ProxiedClient>,
        file_reader: Arc<dyn HubTokenFileReader>,
        clock: Arc<dyn HubClock>,
    ) -> Self {
        let client_id = oidc.device_code_client_id.clone();
        let shared = Arc::new(settings);
        Self {
            oauth: OauthRefresh {
                lock_timeout: LOCK_TIMEOUT,
                hub: Arc::clone(&shared),
                client_id,
                client: Arc::clone(&client),
                clock: Arc::clone(&clock),
                file_reader: Arc::clone(&file_reader),
                state: Mutex::new(State::default()),
            },
            oidc: OidcExchange {
                hub: Arc::clone(&shared),
                settings: oidc,
                client,
                clock,
                cache: Mutex::new(None),
            },
            settings: shared,
            file_reader,
        }
    }

    // huggingface_hub's get_token order: the OIDC exchange, the env token, then the token file with its OAuth refresh
    pub async fn resolve(&self, token: &HubToken) -> Result<Option<SecretString>, HubTokenError> {
        if let HubToken::Explicit(value) = token {
            return Ok(Some(SecretString::from(value.expose_secret().to_owned())));
        }
        if self.settings.disable_implicit_token {
            return Ok(None);
        }
        if let Some(exchanged) = self.oidc.token().await? {
            return Ok(Some(exchanged));
        }
        if let Some(environment_token) = &self.settings.env_token {
            return Ok(Some(SecretString::from(
                environment_token.expose_secret().to_owned(),
            )));
        }
        match read_token_file(&*self.file_reader, &self.settings.token_path).await? {
            Some(file_token) => self.oauth.refresh_if_needed(file_token).await.map(Some),
            None => Ok(None),
        }
    }
}
