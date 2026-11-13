use crate::model_urls::model_name::is_gemini_3_or_newer;

use super::error::GeminiThinkingError;
use super::settings::{MinimalEffortSupport, ThinkingBudgets, ThinkingConfig, ThinkingLevel};

pub fn map_effort_to_budget(
    effort: &str,
    model: Option<&str>,
    budgets: &ThinkingBudgets,
) -> Result<ThinkingConfig, GeminiThinkingError> {
    let (thinking_budget, include_thoughts) = match effort {
        "minimal" => (minimal_budget(model, budgets), true),
        "low" => (budgets.low, true),
        "medium" => (budgets.medium, true),
        "high" => (budgets.high, true),
        "disable" => (budgets.disable, false),
        "none" => (0, false),
        other => {
            return Err(GeminiThinkingError::UnsupportedEffort {
                effort: other.to_owned(),
            });
        }
    };
    Ok(ThinkingConfig {
        include_thoughts: Some(include_thoughts),
        thinking_budget: Some(thinking_budget),
        thinking_level: None,
    })
}

pub fn map_effort_to_level(
    effort: &str,
    model: Option<&str>,
    support: &dyn MinimalEffortSupport,
) -> Result<ThinkingConfig, GeminiThinkingError> {
    let supports_minimal =
        model.is_some_and(|name| !name.is_empty() && supports_minimal_level(name, support));
    let (thinking_level, include_thoughts) = match effort {
        "minimal" => (minimal_level(supports_minimal), true),
        "low" => (ThinkingLevel::Low, true),
        "medium" => (ThinkingLevel::Medium, true),
        "high" => (ThinkingLevel::High, true),
        "disable" | "none" => (minimal_level(supports_minimal), false),
        other => {
            return Err(GeminiThinkingError::UnsupportedEffort {
                effort: other.to_owned(),
            });
        }
    };
    Ok(ThinkingConfig {
        include_thoughts: Some(include_thoughts),
        thinking_budget: None,
        thinking_level: Some(thinking_level),
    })
}

#[must_use]
pub fn supports_minimal_level(model: &str, support: &dyn MinimalEffortSupport) -> bool {
    is_gemini_3_or_newer(model)
        && model.to_lowercase().contains("flash")
        && !support.disables_minimal(model)
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

const fn minimal_level(supports_minimal: bool) -> ThinkingLevel {
    if supports_minimal {
        ThinkingLevel::Minimal
    } else {
        ThinkingLevel::Low
    }
}
