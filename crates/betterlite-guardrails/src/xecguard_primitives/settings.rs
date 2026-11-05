use std::time::Duration;

use secrecy::SecretString;

#[derive(Debug, Clone)]
pub struct XecGuardSettings {
    pub api_key: SecretString,
    pub api_base: String,
    pub model: String,
    pub policy_names: Option<Vec<String>>,
    pub block_on_error: bool,
    pub grounding_strictness: String,
    pub timeout: Option<Duration>,
}
