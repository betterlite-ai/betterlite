use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use thiserror::Error;

use crate::batch::logger::{
    BatchSettings, DEFAULT_FLUSH_INTERVAL_SECONDS, DEFAULT_MAX_QUEUE_SIZE, FLUSH_INTERVAL_VARIABLE,
};

pub const API_KEY_VARIABLE: &str = "POSTHOG_API_KEY";
pub const API_URL_VARIABLE: &str = "POSTHOG_API_URL";
pub const MOCK_VARIABLE: &str = "POSTHOG_MOCK";
pub const DEFAULT_API_URL: &str = "https://us.i.posthog.com";

pub const MAX_BATCH_SIZE: usize = 100;

#[derive(Clone, Debug)]
pub struct PostHogSettings {
    pub api_key: SecretString,
    // LiteLLM keeps the host with its trailing slashes stripped and builds every endpoint from it
    pub api_url: String,
    pub capture_url: String,
    pub batch: BatchSettings,
    pub mock_mode: bool,
}

#[derive(Debug, Error)]
pub enum PostHogSettingsError {
    #[error("POSTHOG_API_KEY is not set, set 'POSTHOG_API_KEY=<>'")]
    MissingApiKey,
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
}
pub fn resolve(
    environment: &dyn EnvironmentLookup,
) -> Result<PostHogSettings, PostHogSettingsError> {
    let api_key = environment
        .lookup(API_KEY_VARIABLE)
        .filter(|value| !value.is_empty())
        .ok_or(PostHogSettingsError::MissingApiKey)?;
    let api_url = environment
        .lookup(API_URL_VARIABLE)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_URL.to_owned());
    let seconds = match environment.lookup(FLUSH_INTERVAL_VARIABLE) {
        Some(text) => betterlite_env::number::<u64>(FLUSH_INTERVAL_VARIABLE, &text)?
            .unwrap_or(DEFAULT_FLUSH_INTERVAL_SECONDS),
        None => DEFAULT_FLUSH_INTERVAL_SECONDS,
    };
    let mock_mode = match environment.lookup(MOCK_VARIABLE) {
        Some(text) => betterlite_env::flag(MOCK_VARIABLE, &text)?.unwrap_or(false),
        None => false,
    };
    let host = api_url.trim_end_matches('/').to_owned();
    Ok(PostHogSettings {
        api_key: SecretString::from(api_key),
        capture_url: format!("{host}/batch/"),
        api_url: host,
        // LiteLLM passes POSTHOG_MAX_BATCH_SIZE, so only the interval follows the batch settings
        batch: BatchSettings {
            batch_size: MAX_BATCH_SIZE,
            flush_interval: Duration::from_secs(seconds),
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
        },
        mock_mode,
    })
}
