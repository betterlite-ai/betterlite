use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::effort::supports_minimal_level;
use super::error::GeminiThinkingError;
use super::settings::{MinimalEffortSupport, ThinkingConfig, ThinkingLevel, ThinkingSettings};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingType {
    Enabled,
    Adaptive,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ThinkingParam {
    #[serde(rename = "type", default)]
    pub kind: Option<ThinkingType>,
    #[serde(default)]
    pub budget_tokens: Option<i64>,
}

#[must_use]
pub fn map_thinking_param(
    param: &ThinkingParam,
    model: Option<&str>,
    settings: &ThinkingSettings,
    support: &dyn MinimalEffortSupport,
) -> ThinkingConfig {
    let enabled = param.kind == Some(ThinkingType::Enabled);
    if model.is_some_and(is_gemini_3_or_newer) {
        let thoughts = enabled && param.budget_tokens != Some(0);
        let mut config = ThinkingConfig {
            include_thoughts: Some(thoughts),
            thinking_budget: None,
            thinking_level: None,
        };
        if thoughts && settings.default_thinking_level_low {
            let minimal = model.is_some_and(|name| supports_minimal_level(name, support));
            config.thinking_level = Some(if minimal {
                ThinkingLevel::Minimal
            } else {
                ThinkingLevel::Low
            });
        }
        return config;
    }
    let mut config = ThinkingConfig::default();
    if enabled && param.budget_tokens != Some(0) {
        config.include_thoughts = Some(true);
    }
    config.thinking_budget = param.budget_tokens;
    config
}

pub fn validate_thinking_conflicts(
    optional_params: &Object,
    param_name: &str,
) -> Result<(), GeminiThinkingError> {
    let Some(Json::Object(existing)) = optional_params.get("thinkingConfig") else {
        return Ok(());
    };
    if existing.contains_key("thinkingLevel") {
        return Err(GeminiThinkingError::ThinkingLevelConflict {
            param: param_name.to_owned(),
        });
    }
    Ok(())
}
