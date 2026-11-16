use secrecy::{ExposeSecret as _, SecretString};

use super::error::LiveSetupError;

pub const DEFAULT_STUDIO_LIVE_BASE: &str = "wss://generativelanguage.googleapis.com";
const LIVE_RPC_PATH: &str =
    "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

pub fn connect_url(
    api_base: Option<&str>,
    explicit_key: Option<&SecretString>,
    environment_key: Option<&SecretString>,
) -> Result<SecretString, LiveSetupError> {
    let base = api_base.unwrap_or(DEFAULT_STUDIO_LIVE_BASE);
    let key = explicit_key
        .or(environment_key)
        .ok_or(LiveSetupError::MissingApiKey)?;
    // WebSocket clients cannot send custom headers on every platform, so the key stays in the query
    let converted = base
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    Ok(SecretString::from(format!(
        "{converted}{LIVE_RPC_PATH}?key={}",
        key.expose_secret()
    )))
}
