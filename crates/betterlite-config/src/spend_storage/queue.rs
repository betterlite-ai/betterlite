use crate::loader::{ConfigError, EnvironmentLookup};

pub(crate) const ENV_SPEND_LOG_QUEUE_MAX_BYTES: &str = "SPEND_LOG_QUEUE_MAX_BYTES";
pub(crate) const DEFAULT_QUEUE_MAX_BYTES: usize = 64_000_000;
pub(crate) const ENV_SPEND_LOGS_URL: &str = "SPEND_LOGS_URL";

// SPEND_LOG_QUEUE_MAX_BYTES is max(1, int(...)) at import, so zero and negative budgets load as one byte
pub(crate) fn max_bytes<L: EnvironmentLookup>(lookup: &L) -> Result<usize, ConfigError> {
    let Some(text) = lookup.lookup(ENV_SPEND_LOG_QUEUE_MAX_BYTES) else {
        return Ok(DEFAULT_QUEUE_MAX_BYTES);
    };
    let number = betterlite_env::number::<i64>(ENV_SPEND_LOG_QUEUE_MAX_BYTES, &text)
        .map_err(|source| ConfigError::Environment { source })?;
    Ok(number.map_or(DEFAULT_QUEUE_MAX_BYTES, |parsed| {
        usize::try_from(parsed.max(1)).unwrap_or(usize::MAX)
    }))
}

// SPEND_LOGS_URL: an empty value counts as unset, so the database branch runs instead
#[must_use]
pub(crate) fn logs_url<L: EnvironmentLookup>(lookup: &L) -> Option<String> {
    lookup
        .lookup(ENV_SPEND_LOGS_URL)
        .filter(|url| !url.is_empty())
}
