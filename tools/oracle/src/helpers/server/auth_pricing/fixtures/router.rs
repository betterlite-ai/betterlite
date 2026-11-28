use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_pricing::cache::ZeroCostCache;
use betterlite_auth::auth_pricing::catalog::{
    ModelGroupCosts, PricingCatalog, PricingDeployment, PricingError,
};
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;
use serde_json::{Value, json};

// what the case scripts on the Router instance: group costs, deployments, resolved model info,
// the raw model_cost map and this instance's zero-cost cache
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouterFixture {
    #[serde(default)]
    pub(crate) groups: BTreeMap<String, GroupFixture>,
    #[serde(default)]
    pub(crate) deployments: BTreeMap<String, Vec<DeploymentFixture>>,
    #[serde(default)]
    pub(crate) resolved: BTreeMap<String, JsonObject>,
    #[serde(default)]
    pub(crate) costs: BTreeMap<String, JsonObject>,
    #[serde(default)]
    pub(crate) cache: Option<BTreeMap<String, bool>>,
    #[serde(default)]
    pub(crate) failing: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupFixture {
    #[serde(default)]
    pub(crate) input: Option<f64>,
    #[serde(default)]
    pub(crate) output: Option<f64>,
}

impl GroupFixture {
    fn costs(&self) -> ModelGroupCosts {
        ModelGroupCosts {
            input_cost_per_token: self.input.map(Money::from_f64),
            output_cost_per_token: self.output.map(Money::from_f64),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentFixture {
    #[serde(default)]
    pub(crate) litellm_params: JsonObject,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
}

impl DeploymentFixture {
    fn deployment(&self) -> PricingDeployment {
        PricingDeployment {
            litellm_params: self.litellm_params.clone(),
            model_info: self.model_info.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ScriptedCatalog {
    groups: BTreeMap<String, ModelGroupCosts>,
    deployments: BTreeMap<String, Vec<PricingDeployment>>,
    resolved: BTreeMap<String, JsonObject>,
    costs: BTreeMap<String, JsonObject>,
    cache: Option<ZeroCostCache>,
    failing: bool,
    lookups: Mutex<Vec<String>>,
}

impl ScriptedCatalog {
    pub(crate) fn new(fixture: &RouterFixture) -> Self {
        let groups = fixture
            .groups
            .iter()
            .map(|(name, group)| (name.clone(), group.costs()))
            .collect();
        let deployments = fixture
            .deployments
            .iter()
            .map(|(name, rows)| {
                let resolved: Vec<PricingDeployment> =
                    rows.iter().map(DeploymentFixture::deployment).collect();
                (name.clone(), resolved)
            })
            .collect();
        let cache = fixture.cache.as_ref().map(|entries| {
            let cache = ZeroCostCache::default();
            for (model, zero_cost) in entries {
                cache.record(model, *zero_cost);
            }
            cache
        });
        Self {
            groups,
            deployments,
            resolved: fixture.resolved.clone(),
            costs: fixture.costs.clone(),
            cache,
            failing: fixture.failing,
            lookups: Mutex::new(Vec::new()),
        }
    }

    // the cache as the call left it, or null when this router instance carries none
    pub(crate) fn cache_value(&self) -> Value {
        self.cache
            .as_ref()
            .map_or(Value::Null, |cache| json!(cache.entries()))
    }

    pub(crate) fn lookups(&self) -> Vec<String> {
        match self.lookups.lock() {
            Ok(lookups) => lookups.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    const fn fail(&self) -> Result<(), PricingError> {
        if self.failing {
            return Err(PricingError::Unavailable);
        }
        Ok(())
    }

    fn recorded(&self, kind: &str, argument: &str) {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{kind}:{argument}"));
    }
}

impl PricingCatalog for ScriptedCatalog {
    fn model_group_info(&self, model: &str) -> Result<Option<ModelGroupCosts>, PricingError> {
        self.recorded("group", model);
        self.fail()?;
        Ok(self.groups.get(model).copied())
    }

    fn model_list(&self, model: &str) -> Result<Vec<PricingDeployment>, PricingError> {
        self.recorded("list", model);
        self.fail()?;
        Ok(self.deployments.get(model).cloned().unwrap_or_default())
    }

    fn deployment_model_info(
        &self,
        model_id: &str,
        model_name: &str,
    ) -> Result<Option<JsonObject>, PricingError> {
        self.recorded("info", &format!("{model_id}|{model_name}"));
        self.fail()?;
        Ok(self.resolved.get(model_id).cloned())
    }

    fn raw_cost_entry(&self, model_id: &str) -> Option<&JsonObject> {
        self.costs.get(model_id)
    }

    fn zero_cost_cache(&self) -> Option<&ZeroCostCache> {
        self.cache.as_ref()
    }
}
