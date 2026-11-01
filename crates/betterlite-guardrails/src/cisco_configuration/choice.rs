use std::env;

use serde::Deserialize;
use serde::de::{self, value::StrDeserializer};

pub const DEFAULT_TIMEOUT_SECONDS: f64 = 10.0_f64;

#[expect(
    clippy::disallowed_methods,
    reason = "Cisco AI Defense guardrail configuration reads its own environment variable \
              fallback directly, mirroring LiteLLM's constructor; budgeted as the one \
              expect task B3759 is allowed."
)]
#[must_use]
pub fn resolve_choice<T>(
    value: Option<&str>,
    env_var: &str,
    allowed: &[&str],
    default: T,
    setting_name: &str,
) -> T
where
    T: Copy + for<'de> Deserialize<'de>,
{
    let candidate = value
        .map(ToOwned::to_owned)
        .or_else(|| env::var(env_var).ok());
    let Some(resolved) = candidate else {
        return default;
    };
    let deserializer = StrDeserializer::<de::value::Error>::new(resolved.as_str());
    T::deserialize(deserializer).unwrap_or_else(|_| {
        tracing::warn!(
            value = resolved.as_str(),
            setting = setting_name,
            allowed = allowed.join(", "),
            "Cisco AI Defense guardrail: invalid config value; falling back to default"
        );
        default
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeoutValue<'timeout> {
    Text(&'timeout str),
    Number(f64),
}

#[must_use]
pub fn coerce_timeout(value: TimeoutValue<'_>) -> Option<f64> {
    let parsed = match value {
        TimeoutValue::Number(number) => Some(number),
        TimeoutValue::Text(text) => text.trim().parse::<f64>().ok(),
    };
    let Some(seconds) = parsed else {
        tracing::warn!(
            setting = "timeout",
            default = DEFAULT_TIMEOUT_SECONDS,
            "Cisco AI Defense guardrail: invalid timeout value; using default"
        );
        return None;
    };
    Some(seconds.clamp(1.0_f64, 60.0_f64))
}

mod tests;
