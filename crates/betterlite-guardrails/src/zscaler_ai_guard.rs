pub mod error;
mod guardrail;
pub mod init;
mod metadata;
mod printed;
pub mod settings;
mod tests;
mod wire;

use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use error::ZscalerAiGuardError;
use settings::Settings;

pub const DEFAULT_URL: &str = "https://api.us1.zseclipse.net/v1/detection/execute-policy";
pub const DEFAULT_TIMEOUT: f64 = 5.0;
pub const ERROR_TYPE: &str = "Zscaler AI Guard Error";
pub const URL_ENV: &str = "ZSCALER_AI_GUARD_URL";
pub const POLICY_ID_ENV: &str = "ZSCALER_AI_GUARD_POLICY_ID";
pub const API_KEY_ENV: &str = "ZSCALER_AI_GUARD_API_KEY";
pub const SEND_ALIAS_ENV: &str = "SEND_USER_API_KEY_ALIAS";
pub const SEND_USER_ID_ENV: &str = "SEND_USER_API_KEY_USER_ID";
pub const SEND_TEAM_ID_ENV: &str = "SEND_USER_API_KEY_TEAM_ID";
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallHook {
    PreCall,
    PostCall,
}

pub const SUPPORTED_EVENT_HOOKS: [CallHook; 2] = [CallHook::PreCall, CallHook::PostCall];

// LiteLLM's GenericGuardrailAPIInputs, holding the texts this guardrail reads and the rest it returns unchanged
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inputs {
    pub texts: Vec<String>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone)]
pub struct ZscalerAiGuard {
    pub guardrail_name: String,
    pub settings: Settings,
    pub client: reqwest::Client,
}

impl ZscalerAiGuard {
    pub fn new<L>(
        args: init::ZscalerArgs,
        environment: &L,
        client: reqwest::Client,
    ) -> Result<Self, ZscalerAiGuardError>
    where
        L: EnvironmentLookup,
    {
        let settings = Settings::resolve(&args, environment)?;
        Ok(Self {
            guardrail_name: args.name,
            settings,
            client,
        })
    }
}
