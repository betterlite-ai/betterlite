use std::time::Duration;

use chrono::{DateTime, Utc};

// RoutingArgs.ttl: the tpm/rpm windows and both per-minute outcome counters expire after a minute
pub const ROUTER_COUNTER_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageKeys {
    pub tpm: String,
    pub rpm: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeKeys {
    pub successes: String,
    pub failures: String,
}

// get_utc_datetime().strftime("%H-%M") keeps the window in UTC whatever the host clock's zone is
#[must_use]
pub fn usage_keys(id: &str, model: &str, now: DateTime<Utc>) -> UsageKeys {
    let minute = now.format("%H-%M").to_string();
    let tpm = format!("global_router:{id}:{model}:tpm:{minute}");
    let rpm = format!("global_router:{id}:{model}:rpm:{minute}");
    UsageKeys { tpm, rpm }
}

#[must_use]
pub fn outcome_keys(id: &str) -> OutcomeKeys {
    let successes = format!("{id}:successes");
    let failures = format!("{id}:fails");
    OutcomeKeys {
        successes,
        failures,
    }
}
