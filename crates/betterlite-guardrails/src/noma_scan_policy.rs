pub mod config;
pub mod error;
pub mod wire;

mod messages;
mod policy;
mod records;
mod request;
mod transport;
mod verdict;

mod tests;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;

use crate::guardrail_records::record::GuardrailStatus;
use wire::ScanResponse;

pub const DEFAULT_API_BASE: &str = "https://api.noma.security/";
pub const DEFAULT_APPLICATION_ID: &str = "litellm";
pub const BLOCKED_MESSAGE: &str = "Request blocked by Noma guardrail";

// The provider name every standard logging record from this guardrail carries; LiteLLM passes "noma".
pub const PROVIDER: &str = "noma";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NomaSettings {
    pub guardrail_name: Option<String>,
    pub event_hook: Option<LitellmParamsMode>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub application_id: Option<String>,
    pub monitor_mode: bool,
    pub anonymize_input: bool,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct NomaScanPolicy {
    pub settings: NomaSettings,
    pub client: reqwest::Client,
    pub now: fn() -> SystemTime,
}

#[derive(Debug, Clone, Copy)]
pub struct ScanCallContext<'context> {
    pub user_auth: &'context BTreeMap<String, GuardrailValue>,
    pub extra_data: &'context BTreeMap<String, GuardrailValue>,
    pub event_type: Option<GuardrailEventHooks>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanOutcome {
    pub status: GuardrailStatus,
    pub response: ScanResponse,
    pub anonymized: Option<String>,
}
