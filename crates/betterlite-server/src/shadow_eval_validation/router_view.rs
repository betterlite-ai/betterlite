use async_trait::async_trait;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_models::decisions::Json;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_catalog::scope::litellm_model;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::listing::get_all_deployments;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_secret_managers::secret_helpers::SecretManagerReadState;
use secrecy::SecretString;

use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::JudgeDispatchError;
use crate::judge_targets::router::JudgeRouter;
use crate::router_dependencies::classify_strategy_router_model;

// The proxy's router as start-time validation reads it: the model list lookups and the strategy-router classification, no dispatch.
pub struct ShadowEvalRouter<'router> {
    pub catalog: &'router DeploymentCatalog,
    pub model_group_alias: &'router AliasMap,
    pub routing_groups: &'router RoutingGroupIndex,
    pub inferred_provider: &'router (dyn Fn(&str) -> Option<String> + Send + Sync),
}

impl ShadowEvalRouter<'_> {
    #[must_use]
    pub fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: self.catalog,
            model_group_alias: self.model_group_alias,
            routing_groups: self.routing_groups,
            inferred_provider: self.inferred_provider,
        }
    }

    // _is_configured_pre_routing_strategy: the registries are keyed by deployment.model_name, so only exact-name rows answer.
    #[must_use]
    pub fn has_strategy_deployment(&self, name: &str) -> bool {
        get_all_deployments(self.catalog, name, None, None)
            .iter()
            .any(|row| {
                matches!(
                    row.litellm_params.get("model"),
                    Some(Json::Text(model)) if classify_strategy_router_model(model).is_some()
                )
            })
    }
}

// resolved_litellm_models: the litellm_params.model of every deployment the name reaches for this team.
#[async_trait]
impl JudgeRouter for ShadowEvalRouter<'_> {
    fn resolved_litellm_models(&self, model: &str, team_id: Option<&str>) -> Vec<String> {
        self.lookup()
            .get_model_list(Some(model), team_id)
            .unwrap_or_default()
            .iter()
            .map(litellm_model)
            .filter(|served| !served.is_empty())
            .map(str::to_owned)
            .collect()
    }

    async fn acompletion(&self, _call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        Err(JudgeDispatchError {
            message: "start-time validation resolves models without dispatching a judge call"
                .to_owned(),
        })
    }
}

// The credential sources _sdk_model_is_missing_anthropic_credentials reads, all passed in by B356.
pub struct AnthropicCredentials<'deps> {
    // litellm.anthropic_key, the module-level SDK key litellm_settings configures
    pub anthropic_key: Option<&'deps SecretString>,
    pub api_key: Option<&'deps SecretString>,
    // AnthropicModelInfo.get_api_key() and get_auth_token(), read from the injected environment by B567
    pub settings: &'deps AnthropicSettings,
    // the hosted manager get_secret would consult, from B4410
    pub secret_state: SecretManagerReadState<'deps>,
}
