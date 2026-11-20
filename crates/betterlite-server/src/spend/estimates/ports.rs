use std::sync::Arc;

use betterlite_config::provider_budget_view::ProviderBudgetConfig;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::provider_budget_view::view::ProviderBudgetReads;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

// The router facts the two endpoints read, so the handlers never hold the router itself.
pub trait EstimateRouterPort: Send + Sync {
    fn deployments(&self) -> &[DeploymentSpec];
    fn model_group_alias(&self) -> &AliasMap;
    fn provider_budget_config(&self) -> Option<&IndexMap<String, ProviderBudgetConfig>>;
    fn budget_limiter(&self) -> Option<Arc<dyn ProviderBudgetReads>>;
}

// The wiring's router adapter: B844's deployment list, the configured aliases, and the provider budget view.
pub struct ConfiguredRouter {
    pub deployments: Vec<DeploymentSpec>,
    pub model_group_alias: AliasMap,
    pub provider_budget_config: Option<IndexMap<String, ProviderBudgetConfig>>,
    pub budget_limiter: Option<Arc<dyn ProviderBudgetReads>>,
}

impl EstimateRouterPort for ConfiguredRouter {
    fn deployments(&self) -> &[DeploymentSpec] {
        &self.deployments
    }

    fn model_group_alias(&self) -> &AliasMap {
        &self.model_group_alias
    }

    fn provider_budget_config(&self) -> Option<&IndexMap<String, ProviderBudgetConfig>> {
        self.provider_budget_config.as_ref()
    }

    fn budget_limiter(&self) -> Option<Arc<dyn ProviderBudgetReads>> {
        self.budget_limiter.as_ref().map(Arc::clone)
    }
}

// The route's wall clock: /provider/budgets stamps its reset times with an injected now.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
