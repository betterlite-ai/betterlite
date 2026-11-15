use std::collections::BTreeMap;

use betterlite_db::spend_batches::StatementLimits;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};

mod tests;

pub const ENV_MAX_BYTES: &str = "SPEND_LOG_WRITE_BATCH_MAX_BYTES";
pub const ENV_MAX_ROWS: &str = "SPEND_LOG_WRITE_BATCH_MAX_ROWS";
pub const DEFAULT_MAX_BYTES: i64 = 2_000_000;
pub const DEFAULT_MAX_ROWS: i64 = 100;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LimitsInput {
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendStatementLimitsError {
    #[error("invalid spend statement limits: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

// LiteLLM reads both settings once at import: max(1, int(os.getenv(name, default)))
pub fn statement_limits<L>(lookup: &L) -> Result<StatementLimits, ConfigError>
where
    L: EnvironmentLookup,
{
    let max_bytes = limit(
        ENV_MAX_BYTES,
        lookup.lookup(ENV_MAX_BYTES).as_deref(),
        DEFAULT_MAX_BYTES,
    )?;
    let max_rows = limit(
        ENV_MAX_ROWS,
        lookup.lookup(ENV_MAX_ROWS).as_deref(),
        DEFAULT_MAX_ROWS,
    )?;
    Ok(StatementLimits {
        max_bytes,
        max_rows,
    })
}

pub fn decode_spend_statement_limits(
    bytes: &[u8],
) -> Result<StatementLimits, SpendStatementLimitsError> {
    let input: LimitsInput = serde_json::from_slice(bytes)?;
    Ok(statement_limits(&input.environment)?)
}

fn limit(name: &str, text: Option<&str>, default: i64) -> Result<usize, ConfigError> {
    let parsed = text.map_or(Ok(default), |text| {
        let value = betterlite_env::number::<i64>(name, text)
            .map_err(|source| ConfigError::Environment { source })?;
        Ok(value.unwrap_or(default))
    })?;
    Ok(usize::try_from(parsed.max(1)).unwrap_or(usize::MAX))
}
