use indexmap::IndexMap;

use super::budget::{
    BudgetDeployment, DeploymentBudget, DeploymentLimits, ProviderBudget, TagBudget,
};
use super::keys;

#[derive(Debug, Default, Eq, PartialEq)]
pub struct CacheKeys {
    pub keys: Vec<String>,
    pub provider_configs: IndexMap<String, ProviderBudget>,
    pub deployment_configs: IndexMap<String, DeploymentBudget>,
    pub deployment_providers: Vec<Option<String>>,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct BudgetWindows {
    pub provider: Option<IndexMap<String, ProviderBudget>>,
    pub deployment: Option<IndexMap<String, DeploymentBudget>>,
    pub tag: Option<IndexMap<String, TagBudget>>,
}

impl BudgetWindows {
    #[must_use]
    pub fn new(
        provider_budget_config: Option<IndexMap<String, ProviderBudget>>,
        tag_budget_config: Option<IndexMap<String, TagBudget>>,
        deployments: &[DeploymentLimits],
    ) -> Self {
        let mut windows = Self {
            provider: provider_budget_config,
            deployment: None,
            tag: tag_budget_config,
        };
        for limits in deployments {
            windows.register_deployment_budget(limits);
        }
        windows
    }

    #[must_use]
    pub fn deployment_budget(&self, model_id: &str) -> Option<&DeploymentBudget> {
        let configs = self.deployment.as_ref()?;
        configs.get(model_id)
    }

    #[must_use]
    pub fn provider_budget(&self, provider: &str) -> Option<&ProviderBudget> {
        let configs = self.provider.as_ref()?;
        configs.get(provider)
    }

    // _init_deployment_budgets and register_deployment_budget register only a complete id, limit and duration
    pub fn register_deployment_budget(&mut self, limits: &DeploymentLimits) {
        let (Some(model_id), Some(max_budget), Some(period)) =
            (&limits.model_id, limits.max_budget, &limits.period)
        else {
            return;
        };
        let configs = self.deployment.get_or_insert_with(IndexMap::new);
        configs.insert(
            model_id.clone(),
            DeploymentBudget {
                max_budget,
                period: period.clone(),
            },
        );
    }

    // Router._sync_deployment_budget_config drops the entry of a deployment whose limits went away
    pub fn sync_deployment_budget(&mut self, limits: &DeploymentLimits) {
        let Some(model_id) = &limits.model_id else {
            return;
        };
        if limits.max_budget.is_none() || limits.period.is_none() {
            self.unregister_deployment_budget(model_id);
            return;
        }
        self.register_deployment_budget(limits);
    }

    pub fn unregister_deployment_budget(&mut self, model_id: &str) {
        let Some(configs) = &mut self.deployment else {
            return;
        };
        configs.shift_remove(model_id);
        if configs.is_empty() {
            self.deployment = None;
        }
    }

    // _async_get_cache_keys_for_router_budget_limiting: one provider key and one deployment key per deployment, then the tag keys
    #[must_use]
    pub fn cache_keys(&self, deployments: &[BudgetDeployment<'_>], tags: &[String]) -> CacheKeys {
        let mut output = CacheKeys::default();
        for deployment in deployments {
            self.push_provider_key(deployment, &mut output);
            self.push_deployment_key(deployment, &mut output);
        }
        for tag in tags {
            if let Some(configs) = &self.tag
                && let Some(budget) = configs.get(tag)
            {
                output.keys.push(keys::tag_keys(tag, &budget.period).spend);
            }
        }
        output
    }

    fn push_provider_key(&self, deployment: &BudgetDeployment<'_>, output: &mut CacheKeys) {
        let Some(configs) = &self.provider else {
            return;
        };
        output
            .deployment_providers
            .push(deployment.provider.map(str::to_owned));
        if let Some(provider) = deployment.provider
            && let Some(budget) = configs.get(provider)
            && budget.period.is_some()
        {
            output
                .keys
                .push(keys::provider_keys(provider, &budget.period).spend);
            output
                .provider_configs
                .insert(provider.to_owned(), budget.clone());
        }
    }

    fn push_deployment_key(&self, deployment: &BudgetDeployment<'_>, output: &mut CacheKeys) {
        if let Some(configs) = &self.deployment
            && let Some(model_id) = deployment.model_id
            && let Some(budget) = configs.get(model_id)
        {
            output
                .keys
                .push(keys::deployment_keys(model_id, &budget.period).spend);
            output
                .deployment_configs
                .insert(model_id.to_owned(), budget.clone());
        }
    }
}

// LiteLLM's should_init_router_budget_limiter: any configured provider or tag map, or a model_list entry with either field
#[must_use]
pub fn should_init_budget_limiter(
    provider_budget_config: Option<&IndexMap<String, ProviderBudget>>,
    tag_budget_config: Option<&IndexMap<String, TagBudget>>,
    model_list: Option<&[DeploymentLimits]>,
) -> bool {
    if provider_budget_config.is_some() || tag_budget_config.is_some() {
        return true;
    }
    let Some(deployments) = model_list else {
        return false;
    };
    deployments
        .iter()
        .any(|limits| limits.max_budget.is_some() || limits.period.is_some())
}
