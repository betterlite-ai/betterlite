use crate::loader::{ConfigError, EnvironmentLookup};

pub const ENV_SPEND_LOG_QUEUE_MAX_BYTES: &str = "SPEND_LOG_QUEUE_MAX_BYTES";
pub const DEFAULT_QUEUE_MAX_BYTES: usize = 64_000_000;
pub const ENV_SPEND_LOGS_URL: &str = "SPEND_LOGS_URL";

// SPEND_LOG_QUEUE_MAX_BYTES is max(1, int(...)) at import, so zero and negative budgets load as one byte
pub fn max_bytes<L: EnvironmentLookup>(lookup: &L) -> Result<usize, ConfigError> {
    let Some(text) = lookup.lookup(ENV_SPEND_LOG_QUEUE_MAX_BYTES) else {
        return Ok(DEFAULT_QUEUE_MAX_BYTES);
    };
    let number = betterlite_env::number::<i64>(ENV_SPEND_LOG_QUEUE_MAX_BYTES, &text)
        .map_err(|source| ConfigError::Environment { source })?;
    Ok(match number {
        Some(parsed) => usize::try_from(parsed.max(1)).unwrap_or(usize::MAX),
        None => DEFAULT_QUEUE_MAX_BYTES,
    })
}

// SPEND_LOGS_URL: an empty value counts as unset, so the database branch runs instead
#[must_use]
pub fn logs_url<L: EnvironmentLookup>(lookup: &L) -> Option<String> {
    lookup
        .lookup(ENV_SPEND_LOGS_URL)
        .filter(|url| !url.is_empty())
}
