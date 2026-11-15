use std::time::Duration;

use rust_decimal::Decimal;

// LiteLLM keeps the configured duration text in every spend key, so the seconds and that text travel together
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetPeriod {
    pub spec: String,
    pub seconds: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderBudget {
    pub budget_limit: Option<Decimal>,
    pub period: Option<BudgetPeriod>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentBudget {
    pub max_budget: Decimal,
    pub period: BudgetPeriod,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagBudget {
    pub max_budget: Option<Decimal>,
    pub period: Option<BudgetPeriod>,
}

// one model_list entry as _init_deployment_budgets and _sync_deployment_budget_config read it
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeploymentLimits {
    pub model_id: Option<String>,
    pub max_budget: Option<Decimal>,
    pub period: Option<BudgetPeriod>,
}

// one healthy deployment as _async_get_cache_keys_for_router_budget_limiting reads it
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BudgetDeployment<'deployment> {
    pub model_id: Option<&'deployment str>,
    pub provider: Option<&'deployment str>,
}
