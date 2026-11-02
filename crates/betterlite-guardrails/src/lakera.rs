use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub const DEFAULT_API_BASE: &str = "https://api.lakera.ai";
pub const DEFAULT_MODERATION_CHECK: ModerationCheck = ModerationCheck::InParallel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationCheck {
    PreCall,
    InParallel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct LakeraCategoryThresholds {
    pub prompt_injection: Option<f64>,
    pub jailbreak: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LakeraEventHook {
    PreCall,
    DuringCall,
}

pub const SUPPORTED_EVENT_HOOKS: [LakeraEventHook; 2] =
    [LakeraEventHook::PreCall, LakeraEventHook::DuringCall];

#[derive(Debug, Default)]
pub struct LegacyLakeraModerationParams {
    pub moderation_check: Option<ModerationCheck>,
    pub category_thresholds: Option<LakeraCategoryThresholds>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug)]
pub struct LegacyLakeraModeration {
    pub moderation_check: ModerationCheck,
    pub category_thresholds: Option<LakeraCategoryThresholds>,
    pub api_base: String,
    pub api_key: SecretString,
    pub async_handler: reqwest::Client,
}

impl LegacyLakeraModeration {
    #[must_use]
    pub fn new(
        params: LegacyLakeraModerationParams,
        get_env: fn(&str) -> String,
        build_client: fn() -> reqwest::Client,
    ) -> Self {
        let api_key = params
            .api_key
            .unwrap_or_else(|| SecretString::from(get_env("LAKERA_API_KEY")));
        let api_base = params.api_base.unwrap_or_else(|| {
            let from_env = get_env("LAKERA_API_BASE");
            if from_env.is_empty() {
                DEFAULT_API_BASE.to_owned()
            } else {
                from_env
            }
        });
        Self {
            moderation_check: params.moderation_check.unwrap_or(DEFAULT_MODERATION_CHECK),
            category_thresholds: params.category_thresholds,
            api_base,
            api_key,
            async_handler: build_client(),
        }
    }
}

#[path = "lakera/tests.rs"]
mod tests;
