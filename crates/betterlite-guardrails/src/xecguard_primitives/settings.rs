use std::time::Duration;

use secrecy::SecretString;

use super::DEFAULT_POLICIES;

pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct XecGuardSettings {
    pub api_key: SecretString,
    pub api_base: String,
    pub model: String,
    pub policy_names: Vec<String>,
    pub block_on_error: bool,
    pub grounding_strictness: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct RawXecGuardSettings {
    pub api_key: SecretString,
    pub api_base: String,
    pub model: String,
    pub policy_names: Option<Vec<String>>,
    pub block_on_error: bool,
    pub grounding_strictness: String,
    pub timeout: Option<Duration>,
}

impl From<RawXecGuardSettings> for XecGuardSettings {
    fn from(raw: RawXecGuardSettings) -> Self {
        Self {
            api_key: raw.api_key,
            api_base: raw.api_base,
            model: raw.model,
            policy_names: raw.policy_names.unwrap_or_else(|| {
                DEFAULT_POLICIES
                    .iter()
                    .map(|policy| (*policy).to_owned())
                    .collect()
            }),
            block_on_error: raw.block_on_error,
            grounding_strictness: raw.grounding_strictness,
            timeout: raw.timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT),
        }
    }
}

mod tests;
