use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_env::EnvError;

use crate::argilla_logging::settings::digit_sampling_rate;
use crate::batch::logger::{
    BATCH_SIZE_VARIABLE as SHARED_BATCH_SIZE_VARIABLE, BatchSettings,
    FLUSH_INTERVAL_VARIABLE as SHARED_INTERVAL_VARIABLE, SettingsError as BatchError,
};
use crate::langsmith_payloads::credentials::{
    CredentialParams, Credentials, get_credentials_from_env,
};
use crate::langsmith_payloads::metadata::DEFAULT_RUN_NAME;

pub const SAMPLING_RATE_VARIABLE: &str = "LANGSMITH_SAMPLING_RATE";
pub const DEFAULT_RUN_NAME_VARIABLE: &str = "LANGSMITH_DEFAULT_RUN_NAME";
pub const BATCH_SIZE_VARIABLE: &str = "LANGSMITH_BATCH_SIZE";

#[derive(Debug)]
pub struct LangsmithSettings {
    pub default_credentials: Credentials,
    pub sampling_rate: f64,
    pub default_run_name: String,
    pub batch: BatchSettings,
    pub redact_user_api_key_info: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("the LangSmith batch settings are invalid")]
    Batch(#[from] BatchError),
    #[error(transparent)]
    BatchSize(#[from] EnvError),
}

pub fn resolve(
    environment: &dyn EnvironmentLookup,
    configured_batch_size: Option<i64>,
    redact_user_api_key_info: bool,
) -> Result<LangsmithSettings, SettingsError> {
    let defaults = BatchSettings::from_environment(&batch_environment(environment))?;
    Ok(LangsmithSettings {
        default_credentials: get_credentials_from_env(
            environment,
            &CredentialParams::default(),
            true,
        ),
        sampling_rate: digit_sampling_rate(environment.lookup(SAMPLING_RATE_VARIABLE).as_deref()),
        default_run_name: environment
            .lookup(DEFAULT_RUN_NAME_VARIABLE)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| DEFAULT_RUN_NAME.to_owned()),
        batch: BatchSettings {
            batch_size: batch_size(environment, configured_batch_size, defaults.batch_size)?,
            flush_interval: defaults.flush_interval,
            max_queue_size: defaults.max_queue_size,
        },
        redact_user_api_key_info,
    })
}

// B256 reads the shared defaults from the process environment, so hand it only the two variables it owns
fn batch_environment(environment: &dyn EnvironmentLookup) -> BTreeMap<String, String> {
    [SHARED_BATCH_SIZE_VARIABLE, SHARED_INTERVAL_VARIABLE]
        .into_iter()
        .filter_map(|name| {
            environment
                .lookup(name)
                .map(|value| (name.to_owned(), value))
        })
        .collect()
}

// LiteLLM's order: LANGSMITH_BATCH_SIZE, then litellm_settings.langsmith_batch_size, then its own default
fn batch_size(
    environment: &dyn EnvironmentLookup,
    configured: Option<i64>,
    default: usize,
) -> Result<usize, SettingsError> {
    if let Some(text) = environment.lookup(BATCH_SIZE_VARIABLE)
        && let Some(size) = betterlite_env::number::<i64>(BATCH_SIZE_VARIABLE, &text)?
    {
        return Ok(flushes_at_least_zero(size));
    }
    configured.map_or(Ok(default), |size| Ok(flushes_at_least_zero(size)))
}

// Python's int() takes a negative size, and len(queue) >= size then flushes on every event, which 0 does too
fn flushes_at_least_zero(size: i64) -> usize {
    match usize::try_from(size.max(0)) {
        Ok(converted) => converted,
        Err(_overflow) => usize::MAX,
    }
}
