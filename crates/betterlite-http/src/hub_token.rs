use std::path::PathBuf;
use std::{io, string};

use secrecy::SecretString;
use thiserror::Error;
use tokio::fs;

use betterlite_config::sections::huggingface_hub::Settings;

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
        source: string::FromUtf8Error,
    },
}

pub struct HubTokenResolver {
    pub settings: Settings,
}

mod tests;

impl HubTokenResolver {
    pub async fn resolve(&self, token: &HubToken) -> Result<Option<SecretString>, HubTokenError> {
        match token {
            HubToken::Explicit(value) => {
                Ok(Some(SecretString::from(value.expose_secret().to_owned())))
            }
            HubToken::Implicit if self.settings.disable_implicit_token => Ok(None),
            HubToken::Implicit => {
                if let Some(environment_token) = &self.settings.env_token {
                    return Ok(Some(SecretString::from(
                        environment_token.expose_secret().to_owned(),
                    )));
                }
                match fs::read(&self.settings.token_path).await {
                    Ok(bytes) => {
                        let text =
                            String::from_utf8(bytes).map_err(|source| HubTokenError::Utf8 {
                                path: self.settings.token_path.clone(),
                                source,
                            })?;
                        let cleaned = text.replace(['\r', '\n'], "").trim().to_owned();
                        Ok((!cleaned.is_empty()).then(|| SecretString::from(cleaned)))
                    }
                    Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
                    Err(source) => Err(HubTokenError::Read {
                        path: self.settings.token_path.clone(),
                        source,
                    }),
                }
            }
        }
    }
}

use secrecy::ExposeSecret as _;
