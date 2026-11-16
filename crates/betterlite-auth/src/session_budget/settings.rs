use std::time::Duration;

use betterlite_env::EnvError;
use betterlite_env::number;

pub const TTL_VARIABLE: &str = "LITELLM_MAX_BUDGET_PER_SESSION_TTL";
pub const DEFAULT_TTL_SECONDS: u64 = 3600;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionBudgetTtlError {
    #[error(transparent)]
    Env(#[from] EnvError),
    #[error("environment variable LITELLM_MAX_BUDGET_PER_SESSION_TTL must not be negative")]
    Negative,
}

// int(os.getenv("LITELLM_MAX_BUDGET_PER_SESSION_TTL", 3600)): unset or empty is the 3600 second default
pub fn session_budget_ttl(text: Option<&str>) -> Result<Duration, SessionBudgetTtlError> {
    let Some(text) = text else {
        return Ok(Duration::from_secs(DEFAULT_TTL_SECONDS));
    };
    let Some(seconds) = number::<i64>(TTL_VARIABLE, text)? else {
        return Ok(Duration::from_secs(DEFAULT_TTL_SECONDS));
    };
    let seconds = u64::try_from(seconds).map_err(|_source| SessionBudgetTtlError::Negative)?;
    Ok(Duration::from_secs(seconds))
}

mod tests;
