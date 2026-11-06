use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use betterlite_config::sections::copilot_device::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use self::decode::{DeviceReply, decode_access_reply, decode_device_reply};
use self::error::{AccessTokenFailure, DeviceAuthError, DeviceCodeFailure};
use self::files::FileAccess;
use self::output::OperatorOutput;
use self::sleeper::Sleeper;
use self::transport::{Post, Transport, TransportFailure};
use crate::openai_base_config::Headers;

pub mod decode;
pub mod error;
pub mod files;
pub mod output;
pub mod sleeper;
mod tests;
pub mod transport;

const LOGIN_ATTEMPTS: u8 = 3;
const POLL_ATTEMPTS: u8 = 12;
const POLL_DELAY: Duration = Duration::from_secs(5);
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Serialize)]
struct DeviceCodeRequest<'request> {
    client_id: &'request str,
    scope: &'request str,
}

#[derive(Serialize)]
struct AccessTokenRequest<'request> {
    client_id: &'request str,
    device_code: &'request str,
    grant_type: &'request str,
}

#[must_use]
pub fn github_headers(access_token: Option<&SecretString>) -> Headers {
    let mut headers = Headers::from([
        (String::from("accept"), String::from("application/json")),
        (
            String::from("editor-version"),
            String::from("vscode/1.85.1"),
        ),
        (
            String::from("editor-plugin-version"),
            String::from("copilot/1.155.0"),
        ),
        (
            String::from("user-agent"),
            String::from("GithubCopilot/1.155.0"),
        ),
        (
            String::from("accept-encoding"),
            String::from("gzip,deflate,br"),
        ),
    ]);
    if let Some(token) = access_token {
        headers.insert(
            String::from("authorization"),
            format!("token {}", token.expose_secret()),
        );
    }
    headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    headers
}

pub struct Authenticator<F, C, T, O> {
    pub settings: Settings,
    pub files: F,
    pub sleeper: C,
    pub transport: T,
    pub output: O,
    pub access_token_path: PathBuf,
    pub api_key_path: PathBuf,
}

impl<F, C, T, O> fmt::Debug for Authenticator<F, C, T, O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authenticator")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl<F, C, T, O> Authenticator<F, C, T, O>
where
    F: FileAccess,
{
    pub fn new(
        settings: Settings,
        mut files: F,
        sleeper: C,
        transport: T,
        output: O,
    ) -> Result<Self, DeviceAuthError> {
        let token_dir = PathBuf::from(&settings.token_dir);
        files
            .ensure_dir(&token_dir)
            .map_err(DeviceAuthError::TokenDirectory)?;
        let access_token_path = token_dir.join(&settings.access_token_file);
        let api_key_path = token_dir.join(&settings.api_key_file);
        Ok(Self {
            settings,
            files,
            sleeper,
            transport,
            output,
            access_token_path,
            api_key_path,
        })
    }
}

fn required_fields(reply: DeviceReply) -> Result<(String, String, String), DeviceCodeFailure> {
    let Some(device_code) = reply.device_code else {
        return Err(DeviceCodeFailure::MissingField {
            name: "device_code",
        });
    };
    let Some(user_code) = reply.user_code else {
        return Err(DeviceCodeFailure::MissingField { name: "user_code" });
    };
    let Some(verification_uri) = reply.verification_uri else {
        return Err(DeviceCodeFailure::MissingField {
            name: "verification_uri",
        });
    };
    Ok((device_code, user_code, verification_uri))
}

enum ReplyFailure {
    Transport(TransportFailure),
    Status(u16),
}

async fn github_reply<T>(
    transport: &mut T,
    url: &str,
    body: Vec<u8>,
) -> Result<Vec<u8>, ReplyFailure>
where
    T: Transport,
{
    let post = Post {
        url,
        headers: github_headers(None),
        body,
    };
    let reply = transport
        .post_json(post)
        .await
        .map_err(ReplyFailure::Transport)?;
    if (200..300).contains(&reply.status) {
        Ok(reply.body)
    } else {
        Err(ReplyFailure::Status(reply.status))
    }
}

impl<F, C, T, O> Authenticator<F, C, T, O>
where
    F: FileAccess,
    C: Sleeper,
    T: Transport,
    O: OperatorOutput,
{
    pub async fn get_access_token(&mut self) -> Result<SecretString, DeviceAuthError> {
        if let Some(token) = self.read_cached_token() {
            return Ok(token);
        }
        for attempt in 1..=LOGIN_ATTEMPTS {
            tracing::debug!(attempt, "acquiring a Copilot access token");
            match self.login().await {
                Ok(token) => {
                    self.save_access_token(&token);
                    return Ok(token);
                }
                Err(error) => {
                    tracing::warn!(attempt, error = %error, "the Copilot login attempt failed");
                }
            }
        }
        Err(DeviceAuthError::Exhausted)
    }

    fn read_cached_token(&mut self) -> Option<SecretString> {
        let Ok(bytes) = self.files.read(&self.access_token_path) else {
            tracing::warn!("no existing Copilot access token was found");
            return None;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            tracing::warn!("the Copilot access token file is not UTF-8");
            return None;
        };
        let token = text.trim();
        if token.is_empty() {
            None
        } else {
            Some(SecretString::from(token))
        }
    }

    fn save_access_token(&mut self, token: &SecretString) {
        if self
            .files
            .write(&self.access_token_path, token.expose_secret().as_bytes())
            .is_err()
        {
            tracing::error!("the Copilot access token could not be saved");
        }
    }

    async fn login(&mut self) -> Result<SecretString, DeviceAuthError> {
        let body = serde_json::to_vec(&DeviceCodeRequest {
            client_id: &self.settings.client_id,
            scope: "read:user",
        })
        .map_err(|source| DeviceAuthError::DeviceCode(DeviceCodeFailure::Encode(source)))?;
        let bytes = github_reply(&mut self.transport, &self.settings.device_code_url, body)
            .await
            .map_err(|failure| {
                DeviceAuthError::DeviceCode(match failure {
                    ReplyFailure::Transport(source) => DeviceCodeFailure::Transport(source),
                    ReplyFailure::Status(status) => DeviceCodeFailure::Status { status },
                })
            })?;
        let reply = decode_device_reply(&bytes)
            .map_err(DeviceCodeFailure::Reply)
            .map_err(DeviceAuthError::DeviceCode)?;
        let (device_code, user_code, verification_uri) =
            required_fields(reply).map_err(DeviceAuthError::DeviceCode)?;
        self.output.instructions(&format!(
            "Please visit {verification_uri} and enter code {user_code} to authenticate."
        ));
        self.poll(&device_code).await
    }

    async fn poll(&mut self, device_code: &str) -> Result<SecretString, DeviceAuthError> {
        for attempt in 1..=POLL_ATTEMPTS {
            let body = serde_json::to_vec(&AccessTokenRequest {
                client_id: &self.settings.client_id,
                device_code,
                grant_type: DEVICE_GRANT_TYPE,
            })
            .map_err(|source| DeviceAuthError::AccessToken(AccessTokenFailure::Encode(source)))?;
            let bytes = github_reply(&mut self.transport, &self.settings.access_token_url, body)
                .await
                .map_err(|failure| {
                    DeviceAuthError::AccessToken(match failure {
                        ReplyFailure::Transport(source) => AccessTokenFailure::Transport(source),
                        ReplyFailure::Status(status) => AccessTokenFailure::Status { status },
                    })
                })?;
            let reply = decode_access_reply(&bytes)
                .map_err(AccessTokenFailure::Reply)
                .map_err(DeviceAuthError::AccessToken)?;
            if let Some(token) = reply.access_token {
                tracing::info!("the GitHub Copilot device login succeeded");
                return Ok(token);
            }
            if reply.error.as_deref() == Some("authorization_pending") {
                tracing::debug!(attempt, "GitHub authorization is pending");
            } else {
                tracing::warn!(attempt, "GitHub returned an unexpected access token reply");
            }
            self.sleeper.sleep(POLL_DELAY).await;
        }
        Err(DeviceAuthError::AccessToken(AccessTokenFailure::Timeout))
    }
}
