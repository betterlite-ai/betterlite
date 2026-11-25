use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::types::{Config, ModelDeployment};
use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::inferred::inferred_provider;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{self, AliasMap};
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::error::RoutingGroupError;
use betterlite_router::routing_groups::group::RoutingGroup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::wildcard_catalog::error::PatternCatalogError;

use self::pricing::extract_custom_pricing;
use crate::cost_service::CostService;

mod pricing;

// The router snapshot the quote's model resolution reads, built from the same B844 indices the server serves.
pub struct DeploymentSet {
    pub(crate) catalog: DeploymentCatalog,
    pub(crate) aliases: AliasMap,
    pub(crate) groups: RoutingGroupIndex,
}

#[derive(Debug, thiserror::Error)]
pub enum DeploymentSetError {
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    Pattern(#[from] PatternCatalogError),
    #[error(transparent)]
    Group(#[from] RoutingGroupError),
    #[error("model_list entry {index} is not an object")]
    Record { index: usize },
    #[error("model_list entry {index} has no model_name")]
    Name { index: usize },
}

// The model a quote prices, its explicit provider, and the deployment prices that stand in for the cost map.
pub struct ResolvedCostModel {
    pub model: String,
    pub provider: Option<String>,
    pub custom_cost_per_token: Option<CustomCostPerToken>,
}

impl DeploymentSet {
    pub fn from_config(config: &Config) -> Result<Self, DeploymentSetError> {
        let mut catalog = DeploymentCatalog::default();
        for (index, deployment) in config.model_list.iter().enumerate() {
            let provider = config
                .sections
                .provider_resolution
                .deployments
                .get(index)
                .and_then(Option::as_deref);
            let spec = Self::config_spec(deployment, provider)?;
            index_deployment(&mut catalog, spec)?;
        }
        let groups = routing_groups(config, &catalog.model_names)?;
        let aliases = alias::from_config(&config.sections.deployment_catalog.model_group_alias);
        Ok(Self {
            catalog,
            aliases,
            groups,
        })
    }

    // Deployment.__init__ over one config row: its params, its info id, and the provider the config section resolved.
    fn config_spec(
        deployment: &ModelDeployment,
        provider: Option<&str>,
    ) -> Result<DeploymentSpec, InvalidDeployment> {
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
        DeploymentSpec::new(
            deployment.model_name.clone(),
            params,
            info,
            JsonObject::new(),
            || uuid::Uuid::new_v4().to_string(),
        )
    }

    pub fn from_records(rows: &[JsonObject]) -> Result<Self, DeploymentSetError> {
        let mut catalog = DeploymentCatalog::default();
        for (index, row) in rows.iter().enumerate() {
            let Some(Json::Text(name)) = row.get("model_name") else {
                return Err(DeploymentSetError::Name { index });
            };
            let params = match row.get("litellm_params") {
                None | Some(Json::Null) => JsonObject::new(),
                Some(Json::Object(params)) => params.clone(),
                Some(_) => return Err(DeploymentSetError::Record { index }),
            };
            let info = match row.get("model_info") {
                Some(Json::Object(info)) => info.clone(),
                _ => JsonObject::new(),
            };
            let spec = DeploymentSpec::new(name.clone(), params, info, JsonObject::new(), || {
                uuid::Uuid::new_v4().to_string()
            })?;
            index_deployment(&mut catalog, spec)?;
        }
        Ok(Self {
            catalog,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
        })
    }

    // _resolve_model_for_cost_lookup: the first listed deployment answers, or the requested name when nothing matches.
    #[must_use]
    pub fn resolve(
        &self,
        service: &CostService<ProviderFormulas>,
        requested: &str,
    ) -> ResolvedCostModel {
        let resolution = service.lookup.resolution();
        let inferred = |model: &str| {
            inferred_provider(model, &resolution).map(|identity| identity.slug().to_owned())
        };
        let lookup = CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &inferred,
        };
        let Ok(rows) = lookup.get_model_list(Some(requested), None) else {
            return unresolvable(requested);
        };
        let Some(first) = rows.first() else {
            return unresolvable(requested);
        };
        resolved_row(service, first).unwrap_or_else(|| unresolvable(requested))
    }
}

#[must_use]
pub fn unresolvable(requested: &str) -> ResolvedCostModel {
    ResolvedCostModel {
        model: requested.to_owned(),
        provider: None,
        custom_cost_per_token: None,
    }
}

fn resolved_row(
    service: &CostService<ProviderFormulas>,
    row: &DeploymentSpec,
) -> Option<ResolvedCostModel> {
    let provider = match row.litellm_params.get("custom_llm_provider") {
        Some(Json::Text(provider)) => Some(provider.clone()),
        _ => None,
    };
    let model = text_field(&row.model_info, "base_model")
        .or_else(|| text_field(&row.litellm_params, "base_model"))
        .or_else(|| text_field(&row.litellm_params, "model"))
        .map(str::to_owned)?;
    let custom = extract_custom_pricing(
        &row.litellm_params,
        &row.model_info,
        service,
        &model,
        provider.as_deref(),
    );
    Some(ResolvedCostModel {
        model,
        provider,
        custom_cost_per_token: custom,
    })
}

// An empty value answers like an absent one, as the source's `or` chain reads it.
fn text_field<'fields>(fields: &'fields JsonObject, name: &str) -> Option<&'fields str> {
    match fields.get(name) {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    }
}

// the routing group index Router._init_routing_groups builds, shared with the deployment affinity check
pub(crate) fn routing_groups(
    config: &Config,
    names: &BTreeSet<String>,
) -> Result<RoutingGroupIndex, DeploymentSetError> {
    let Some(settings) = config.sections.routing_groups.as_ref() else {
        return Ok(RoutingGroupIndex::default());
    };
    let mut parsed = Vec::with_capacity(settings.groups.len());
    for group in &settings.groups {
        parsed.push(RoutingGroup::new(
            group.group_name.clone(),
            group.models.clone(),
            &group.routing_strategy,
            group.routing_strategy_args.clone(),
            group.model_priorities.clone(),
        )?);
    }
    let aliases: BTreeMap<String, String> = config
        .sections
        .deployment_catalog
        .model_group_alias
        .iter()
        .map(|(name, entry)| (name.clone(), entry.model.clone()))
        .collect();
    Ok(RoutingGroupIndex::build(parsed, names, &aliases)?.index)
}

fn index_deployment(
    catalog: &mut DeploymentCatalog,
    spec: DeploymentSpec,
) -> Result<(), DeploymentSetError> {
    catalog.index_patterns(&spec)?;
    catalog.model_names.insert(spec.model_name.clone());
    catalog.push(spec);
    Ok(())
}
