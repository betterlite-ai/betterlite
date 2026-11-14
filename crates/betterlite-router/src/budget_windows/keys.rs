use super::budget::BudgetPeriod;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowKeys {
    pub spend: String,
    pub start_time: String,
}

// LiteLLM interpolates the configured duration text into the spend key, so "1d" and "24h" name different windows
#[must_use]
pub const fn period_label(period: &Option<BudgetPeriod>) -> &str {
    let Some(configured) = period else {
        return "None";
    };
    configured.spec.as_str()
}

#[must_use]
pub fn provider_keys(provider: &str, period: &Option<BudgetPeriod>) -> WindowKeys {
    let spend = format!("provider_spend:{provider}:{}", period_label(period));
    let start_time = format!("provider_budget_start_time:{provider}");
    WindowKeys { spend, start_time }
}

#[must_use]
pub fn deployment_keys(model_id: &str, period: &BudgetPeriod) -> WindowKeys {
    let spend = format!("deployment_spend:{model_id}:{}", period.spec);
    let start_time = format!("deployment_budget_start_time:{model_id}");
    WindowKeys { spend, start_time }
}

#[must_use]
pub fn tag_keys(tag: &str, period: &Option<BudgetPeriod>) -> WindowKeys {
    let spend = format!("tag_spend:{tag}:{}", period_label(period));
    let start_time = format!("tag_budget_start_time:{tag}");
    WindowKeys { spend, start_time }
}
