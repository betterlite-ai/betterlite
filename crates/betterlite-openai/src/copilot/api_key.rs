use secrecy::{ExposeSecret as _, SecretString};

use self::clock::Clock;
use self::decode::{CopilotKeyRecord, decode_copilot_key_record};
use self::error::ApiKeyError;
use self::transport::{ApiKeyTransport, Get};
use super::device_auth::Authenticator;
use super::device_auth::files::FileAccess;
use super::device_auth::github_headers;
use super::device_auth::output::OperatorOutput;
use super::device_auth::sleeper::Sleeper;
use super::device_auth::transport::Transport;
use crate::openai_base_config::Headers;

pub mod clock;
pub mod decode;
pub mod error;
mod tests;
pub mod transport;

const REFRESH_ATTEMPTS: u8 = 3;

impl<F, C, T, O> Authenticator<F, C, T, O>
where
    F: FileAccess,
    C: Clock + Sleeper,
    T: ApiKeyTransport + Transport,
    O: OperatorOutput,
{
    pub async fn get_api_key(&mut self) -> Result<Option<SecretString>, ApiKeyError> {
        let cached = self.files.read(&self.api_key_path).await;
        match cached {
            Ok(bytes) => match decode_copilot_key_record(&bytes) {
                Ok(record)
                    if record
                        .expires_at
                        .is_some_and(|expires| expires > self.sleeper.now()) =>
                {
                    return Ok(record.token);
                }
                Ok(_) => tracing::warn!("the Copilot API key expired; refreshing it"),
                Err(error) => {
                    tracing::warn!(error = %error, "the Copilot API key file could not be decoded");
                }
            },
            Err(error) => tracing::warn!(error = %error, "no Copilot API key file was found"),
        }
        let record = self.refresh_api_key().await?;
        let bytes = serde_json::to_vec(&record).map_err(ApiKeyError::Encode)?;
        if let Err(source) = self.files.write(&self.api_key_path, &bytes).await {
            tracing::error!(error = %source, "the Copilot API key record could not be saved");
            return Err(ApiKeyError::Save(source));
        }
        match record.token {
            Some(token) if !token.expose_secret().is_empty() => Ok(Some(token)),
            _ => Err(ApiKeyError::MissingToken),
        }
    }

    pub async fn get_api_base(&mut self) -> Option<String> {
        let read = self.files.read(&self.api_key_path).await;
        match read {
            Ok(bytes) => match decode_copilot_key_record(&bytes) {
                Ok(record) => record.endpoints.and_then(|endpoints| endpoints.api),
                Err(error) => {
                    tracing::warn!(error = %error, "the Copilot API key file could not be decoded");
                    None
                }
            },
            Err(error) => {
                tracing::warn!(error = %error, "no Copilot API key file was found");
                None
            }
        }
    }

    async fn refresh_api_key(&mut self) -> Result<CopilotKeyRecord, ApiKeyError> {
        let access_token = self
            .get_access_token()
            .await
            .map_err(ApiKeyError::AccessToken)?;
        let headers = github_headers(Some(&access_token));
        for attempt in 1..=REFRESH_ATTEMPTS {
            if let Some(record) = self.fetch_api_key_record(&headers, attempt).await {
                return Ok(record);
            }
        }
        Err(ApiKeyError::Refresh)
    }

    async fn fetch_api_key_record(
        &mut self,
        headers: &Headers,
        attempt: u8,
    ) -> Option<CopilotKeyRecord> {
        let get = Get {
            url: &self.settings.api_key_url,
            headers: headers.clone(),
        };
        let reply = match self.transport.get(get).await {
            Ok(reply) => reply,
            Err(error) => {
                tracing::warn!(attempt, error = %error, "the Copilot API key request could not be sent");
                return None;
            }
        };
        if !(200..300).contains(&reply.status) {
            tracing::warn!(
                attempt,
                status = reply.status,
                "the Copilot API key request failed"
            );
            return None;
        }
        match decode_copilot_key_record(&reply.body) {
            Ok(record) if record.token.is_some() => Some(record),
            Ok(_) => {
                tracing::warn!(attempt, "the Copilot API key reply has no token");
                None
            }
            Err(error) => {
                tracing::warn!(attempt, error = %error, "the Copilot API key reply could not be decoded");
                None
            }
        }
    }
}
