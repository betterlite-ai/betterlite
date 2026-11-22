use betterlite_env::{EnvError, number};

use crate::loader::EnvironmentLookup;

mod tests;

pub const MAX_ATTEMPTS_ENV: &str = "RESPONSES_SESSION_LOOKUP_MAX_ATTEMPTS";
pub const RETRY_INTERVAL_ENV: &str = "RESPONSES_SESSION_LOOKUP_RETRY_INTERVAL";

// constants.py clamps the attempts to at least one but keeps the delay as given, a negative one included
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionLookupSettings {
    pub max_attempts: u64,
    pub retry_interval_seconds: f64,
}

pub fn loads_session_lookup_settings(
    environment: &dyn EnvironmentLookup,
) -> Result<SessionLookupSettings, EnvError> {
    let attempts_text = environment.lookup(MAX_ATTEMPTS_ENV).unwrap_or_default();
    let attempts = number::<i64>(MAX_ATTEMPTS_ENV, &attempts_text)?.unwrap_or(3_i64);
    let interval_text = environment.lookup(RETRY_INTERVAL_ENV).unwrap_or_default();
    let interval = number::<f64>(RETRY_INTERVAL_ENV, &interval_text)?.unwrap_or(0.2_f64);
    Ok(SessionLookupSettings {
        max_attempts: attempts.max(1_i64).unsigned_abs(),
        retry_interval_seconds: interval,
    })
}
