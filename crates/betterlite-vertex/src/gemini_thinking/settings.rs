use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub struct ThinkingBudgets {
    pub minimal: i64,
    pub minimal_flash: i64,
    pub minimal_pro: i64,
    pub minimal_flash_lite: i64,
    pub low: i64,
    pub medium: i64,
    pub high: i64,
    pub disable: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub struct ThinkingSettings {
    pub budgets: ThinkingBudgets,
    pub default_thinking_level_low: bool,
}

pub trait MinimalEffortSupport {
    fn disables_minimal(&self, model: &str) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_thoughts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
}
