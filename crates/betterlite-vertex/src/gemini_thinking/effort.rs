use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::error::GeminiThinkingError;
use super::settings::{MinimalEffortSupport, ThinkingBudgets, ThinkingConfig, ThinkingLevel};

pub fn map_effort_to_budget(
    effort: &str,
    model: Option<&str>,
    budgets: &ThinkingBudgets,
) -> Result<ThinkingConfig, GeminiThinkingError> {
    let mut config = base_config(effort)?;
    config.thinking_budget = Some(effort_budget(effort, model, budgets));
    Ok(config)
}

pub fn map_effort_to_level(
    effort: &str,
    model: Option<&str>,
    support: &dyn MinimalEffortSupport,
) -> Result<ThinkingConfig, GeminiThinkingError> {
    let base = base_config(effort)?;
    let minimal = model.is_some_and(|name| supports_minimal_level(name, support));
    let thinking_level = match effort {
        "medium" => ThinkingLevel::Medium,
        "high" => ThinkingLevel::High,
        "minimal" | "disable" | "none" if minimal => ThinkingLevel::Minimal,
        _ => ThinkingLevel::Low,
    };
    Ok(ThinkingConfig {
        thinking_level: Some(thinking_level),
        ..base
    })
}

#[must_use]
pub fn supports_minimal_level(model: &str, support: &dyn MinimalEffortSupport) -> bool {
    is_gemini_3_or_newer(model)
        && model.to_lowercase().contains("flash")
        && !support.disables_minimal(model)
}

fn base_config(effort: &str) -> Result<ThinkingConfig, GeminiThinkingError> {
    let include_thoughts = match effort {
        "minimal" | "low" | "medium" | "high" => true,
        "disable" | "none" => false,
        other => {
            return Err(GeminiThinkingError::UnsupportedEffort {
                effort: other.to_owned(),
            });
        }
    };
    Ok(ThinkingConfig {
        include_thoughts: Some(include_thoughts),
        thinking_budget: None,
        thinking_level: None,
    })
}

fn effort_budget(effort: &str, model: Option<&str>, budgets: &ThinkingBudgets) -> i64 {
    match effort {
        "minimal" => minimal_budget(model, budgets),
        "disable" => budgets.disable,
        "low" => budgets.low,
        "medium" => budgets.medium,
        "high" => budgets.high,
        _ => 0,
    }
}

fn minimal_budget(model: Option<&str>, budgets: &ThinkingBudgets) -> i64 {
    let lowered = model.unwrap_or_default().to_lowercase();
    if lowered.contains("gemini-2.5-flash-lite") {
        budgets.minimal_flash_lite
    } else if lowered.contains("gemini-2.5-pro") {
        budgets.minimal_pro
    } else if lowered.contains("gemini-2.5-flash") {
        budgets.minimal_flash
    } else {
        budgets.minimal
    }
}
