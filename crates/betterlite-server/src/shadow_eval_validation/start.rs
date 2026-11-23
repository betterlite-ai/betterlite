use std::collections::BTreeSet;

use betterlite_config::types::{Config, ModelDeployment};
use betterlite_models::decisions::Json;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::ResolutionContext;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{self, AliasMap};
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use uuid::Uuid;

use crate::cost_estimate_route::deployments::{DeploymentSetError, routing_groups};
use crate::judge_targets::sdk::JudgeSdk;
use crate::shadow_eval_validation::error::ShadowEvalValidationError;
use crate::shadow_eval_validation::models::{
    reject_judge_collision, validate_model_scope, validate_plain_model,
};
use crate::shadow_eval_validation::request::StartShadowEvalRequest;
use crate::shadow_eval_validation::router_view::{AnthropicCredentials, ShadowEvalRouter};

// The proxy's configured deployments as start validation reads them, built from the same indices the cost quote serves.
pub struct ShadowCatalog {
    pub catalog: DeploymentCatalog,
    pub aliases: AliasMap,
    pub groups: RoutingGroupIndex,
}

impl ShadowCatalog {
    // a row that cannot be indexed fails the whole build, as the cost quote's own deployment set does
    pub fn from_config(config: &Config) -> Result<Self, DeploymentSetError> {
        let mut catalog = DeploymentCatalog::default();
        for (index, deployment) in config.model_list.iter().enumerate() {
            let provider = config
                .sections
                .provider_resolution
                .deployments
                .get(index)
                .and_then(Option::as_deref);
            let spec = deployment_spec(deployment, provider)?;
            catalog.index_patterns(&spec)?;
            catalog.model_names.insert(spec.model_name.clone());
            catalog.push(spec);
        }
        let groups = routing_groups(config, &catalog.model_names)?;
        let aliases = alias::from_config(&config.sections.deployment_catalog.model_group_alias);
        Ok(Self {
            catalog,
            aliases,
            groups,
        })
    }
}

// Deployment.__init__ over one config row: its params, its info id, and the provider the config resolved.
fn deployment_spec(
    deployment: &ModelDeployment,
    provider: Option<&str>,
) -> Result<DeploymentSpec, DeploymentSetError> {
    let mut params = JsonObject::new();
    if let Some(model) = deployment.litellm_params.model.as_ref() {
        params.insert("model".to_owned(), Json::Text(model.clone()));
    }
    if let Some(slug) = provider {
        params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(slug.to_owned()),
        );
    }
    let mut info = JsonObject::new();
    if let Some(id) = deployment.model_info.id.as_ref() {
        info.insert("id".to_owned(), Json::Text(id.clone()));
    }
    Ok(DeploymentSpec::new(
        deployment.model_name.clone(),
        params,
        info,
        JsonObject::new(),
        || Uuid::new_v4().to_string(),
    )?)
}

pub struct TargetKey<'rows> {
    pub token: &'rows str,
    pub team_id: Option<&'rows str>,
}

#[derive(Clone, Copy)]
pub struct TargetRecords<'rows> {
    pub keys: &'rows [TargetKey<'rows>],
    pub teams: &'rows [&'rows str],
    pub users: &'rows [&'rows str],
}

pub struct StartContext<'deps> {
    pub router: Option<ShadowEvalRouter<'deps>>,
    pub sdk: &'deps dyn JudgeSdk,
    pub cache: &'deps ProviderCache,
    pub resolution: &'deps ResolutionContext<'deps>,
    pub credentials: AnthropicCredentials<'deps>,
    pub records: TargetRecords<'deps>,
}

pub fn validate_start(
    request: &StartShadowEvalRequest,
    context: &StartContext<'_>,
) -> Result<(), ShadowEvalValidationError> {
    reject_unconfigured_routers(request, context)?;
    reject_unknown_targets(request, &context.records)?;
    let team_ids = resolved_team_ids(request, &context.records);
    validate_plain_model(context, &request.judge_model, "judge_model", &team_ids)?;
    if let Some(baseline) = request.baseline_model.as_deref() {
        validate_plain_model(context, baseline, "baseline_model", &team_ids)?;
    }
    reject_judge_collision(context, request, &team_ids)?;
    validate_model_scope(context, &request.models)
}

fn reject_unconfigured_routers(
    request: &StartShadowEvalRequest,
    context: &StartContext<'_>,
) -> Result<(), ShadowEvalValidationError> {
    let unconfigured: Vec<&String> = request
        .router_names
        .iter()
        .filter(|name| {
            !context
                .router
                .as_ref()
                .is_some_and(|router| router.has_strategy_deployment(name.as_str()))
        })
        .collect();
    if unconfigured.is_empty() {
        return Ok(());
    }
    let names = unconfigured
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<String>>()
        .join(", ");
    Err(ShadowEvalValidationError::UnconfiguredRouters { names })
}

fn reject_unknown_targets(
    request: &StartShadowEvalRequest,
    records: &TargetRecords<'_>,
) -> Result<(), ShadowEvalValidationError> {
    let known_keys: BTreeSet<&str> = records.keys.iter().map(|key| key.token).collect();
    let known_teams: BTreeSet<&str> = records.teams.iter().copied().collect();
    let known_users: BTreeSet<&str> = records.users.iter().copied().collect();
    let faults: Vec<String> = [
        (
            "api_key_ids",
            unknown_ids(&request.api_key_ids, &known_keys),
        ),
        ("team_ids", unknown_ids(&request.team_ids, &known_teams)),
        ("user_ids", unknown_ids(&request.user_ids, &known_users)),
    ]
    .into_iter()
    .filter_map(|(field, ids)| ids.map(|missing| format!("{field} not on this proxy: {missing}")))
    .collect();
    if faults.is_empty() {
        return Ok(());
    }
    let message = faults
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(". ");
    Err(ShadowEvalValidationError::UnknownTargets { message })
}

fn unknown_ids(requested: &[String], known: &BTreeSet<&str>) -> Option<String> {
    let mut unknown: Vec<&str> = requested
        .iter()
        .map(String::as_str)
        .filter(|id| !known.contains(id))
        .collect();
    unknown.sort_unstable();
    (!unknown.is_empty()).then(|| unknown.join(", "))
}

fn resolved_team_ids(
    request: &StartShadowEvalRequest,
    records: &TargetRecords<'_>,
) -> Vec<Option<String>> {
    let mut teams: Vec<Option<String>> = Vec::new();
    if !request.api_key_ids.is_empty() {
        teams.extend(
            records
                .keys
                .iter()
                .map(|key| key.team_id.map(str::to_owned)),
        );
    }
    teams.extend(request.team_ids.iter().cloned().map(Some));
    if !request.user_ids.is_empty() {
        teams.push(None);
    }
    let mut seen = BTreeSet::new();
    teams
        .into_iter()
        .filter(|team| seen.insert(team.clone()))
        .collect()
}
