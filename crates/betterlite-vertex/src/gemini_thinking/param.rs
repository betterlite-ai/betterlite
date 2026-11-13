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
    let thoughts = param.kind == Some(ThinkingType::Enabled) && param.budget_tokens != Some(0);
    if let Some(name) = model.filter(|name| is_gemini_3_or_newer(name)) {
        let default_level = if supports_minimal_level(name, support) {
            ThinkingLevel::Minimal
        } else {
            ThinkingLevel::Low
        };
        return ThinkingConfig {
            include_thoughts: Some(thoughts),
            thinking_budget: None,
            thinking_level: (thoughts && settings.default_thinking_level_low)
                .then_some(default_level),
        };
    }
    ThinkingConfig {
        include_thoughts: thoughts.then_some(true),
        thinking_budget: param.budget_tokens,
        thinking_level: None,
    }
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
