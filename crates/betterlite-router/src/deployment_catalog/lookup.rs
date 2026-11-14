use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_catalog::scope::{across_teams, all_deployments, litellm_model};
use crate::deployment_catalog::types::{
    AliasSource, AliasStep, CandidateView, Candidates, CatalogMatch, LookupRequest,
};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::wildcard_catalog::PatternCatalog;

const STRATEGY_MARKER_PREFIX: &str = "auto_router/";

// Router._drop_strategy_markers: a strategy marker is never a callable deployment.
fn drop_markers(
    model: &str,
    deployments: Vec<DeploymentSpec>,
) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
    let had_any = !deployments.is_empty();
    let selectable: Vec<DeploymentSpec> = deployments
        .into_iter()
        .filter(|spec| !litellm_model(spec).starts_with(STRATEGY_MARKER_PREFIX))
        .collect();
    if had_any && selectable.is_empty() {
        return Err(CatalogLookupError::OnlyStrategyMarkers {
            model: model.to_owned(),
        });
    }
    Ok(selectable)
}

fn matched(
    group: String,
    found: Vec<DeploymentSpec>,
    aliases: Vec<AliasStep>,
) -> Result<CatalogMatch, CatalogLookupError> {
    let pool = drop_markers(&group, found)?;
    Ok(CatalogMatch {
        group,
        candidates: Candidates::Pool(pool),
        aliases,
    })
}

// The snapshot, the alias maps and the caller's authorized view for one lookup.
pub struct CatalogResolver<'catalog> {
    pub catalog: &'catalog DeploymentCatalog,
    pub model_group_alias: &'catalog BTreeMap<String, String>,
    pub model_alias_map: &'catalog BTreeMap<String, String>,
    pub view: &'catalog dyn CandidateView,
}

impl CatalogResolver<'_> {
    // resolve_model_group_alias: a target that is empty rewrites nothing.
    fn group_alias(&self, model: &str) -> Option<&str> {
        self.model_group_alias
            .get(model)
            .map(String::as_str)
            .filter(|target| !target.is_empty())
    }

    // Router._get_deployment_by_litellm_model
    fn by_litellm_model(&self, model: &str) -> Vec<DeploymentSpec> {
        self.catalog
            .models
            .iter()
            .filter(|spec| litellm_model(spec) == model)
            .cloned()
            .collect()
    }

    // Router._filter_reserved_deployments
    fn reserved(
        &self,
        model: &str,
        deployments: Vec<DeploymentSpec>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        self.view
            .filter_reserved(drop_markers(model, deployments)?, team_id)
    }

    fn wildcard_pool(
        &self,
        router: &PatternCatalog,
        model: &str,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        let provider = self.view.inferred_provider(model);
        let candidates = router.get_deployments_by_pattern(model, provider.as_deref());
        let mut pool = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let registered = candidate
                .model_id
                .as_deref()
                .and_then(|id| self.catalog.id_index.get(id))
                .and_then(|index| self.catalog.models.get(*index))
                .ok_or_else(|| CatalogLookupError::UnindexedCandidate {
                    model_id: candidate.model_id.clone(),
                })?;
            let mut spec = registered.clone();
            spec.litellm_params
                .insert("model".to_owned(), Json::Text(candidate.model));
            pool.push(spec);
        }
        Ok(pool)
    }

    // Router._try_early_resolve_deployments_for_model_not_in_names; default_deployment is commented out at the pin.
    fn early_resolve(
        &self,
        model: &str,
        team_id: Option<&str>,
        proxy_admin: bool,
    ) -> Result<Option<Vec<DeploymentSpec>>, CatalogLookupError> {
        if self.catalog.model_names.contains(model) {
            return Ok(None);
        }
        let named = match team_id {
            Some(team) => all_deployments(self.catalog, model, Some(team)),
            None if proxy_admin => across_teams(self.catalog, model)?,
            None => Vec::new(),
        };
        if !named.is_empty() {
            return Ok(Some(named));
        }
        let mut pool = self.wildcard_pool(&self.catalog.pattern_router, model)?;
        if pool.is_empty()
            && let Some(patterns) =
                team_id.and_then(|team| self.catalog.team_pattern_routers.get(team))
        {
            pool = self.wildcard_pool(&patterns.router, model)?;
        }
        Ok(Some(pool).filter(|found| !found.is_empty()))
    }

    // Router.deployments_for_request
    pub fn deployments_for_request(
        &self,
        request: &LookupRequest<'_>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        let name = self.group_alias(request.model).unwrap_or(request.model);
        let team_id = request.authority.team_id.filter(|team| !team.is_empty());
        let found = all_deployments(self.catalog, name, team_id);
        if !found.is_empty() || team_id.is_some() || !request.authority.proxy_admin {
            return Ok(found);
        }
        across_teams(self.catalog, name)
    }

    // The model id arm of Router._common_checks_available_deployment: a model that is no group name but a deployment id.
    fn by_deployment_id(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<Option<CatalogMatch>, CatalogLookupError> {
        if self.catalog.model_names.contains(model) {
            return Ok(None);
        }
        let Some(registered) = self
            .catalog
            .id_index
            .get(model)
            .and_then(|index| self.catalog.models.get(*index))
        else {
            return Ok(None);
        };
        let group = litellm_model(registered).to_owned();
        let kept = self.reserved(&group, vec![registered.clone()], team_id)?;
        let aliases = vec![AliasStep {
            source: AliasSource::DeploymentId,
            from: model.to_owned(),
            to: group.clone(),
        }];
        // LiteLLM indexes [0] of this list and fails with a 500 when the reservation filter empties it
        let Some(only) = kept.into_iter().next() else {
            return Err(CatalogLookupError::NoHealthyDeployments { model: group });
        };
        Ok(Some(CatalogMatch {
            group,
            candidates: Candidates::Single(Box::new(only)),
            aliases,
        }))
    }

    // Router._common_checks_available_deployment
    pub fn resolve(&self, request: &LookupRequest<'_>) -> Result<CatalogMatch, CatalogLookupError> {
        let team_id = request.authority.team_id.filter(|team| !team.is_empty());
        let mut model = request.model.to_owned();
        let mut aliases = Vec::new();
        if request.specific_deployment {
            let found = self.reserved(&model, self.by_litellm_model(&model), team_id)?;
            return matched(model, found, aliases);
        }
        if let Some(found) = self.by_deployment_id(&model, team_id)? {
            return Ok(found);
        }
        if let Some(target) = self.group_alias(&model) {
            aliases.push(AliasStep {
                source: AliasSource::ModelGroupAlias,
                from: model.clone(),
                to: target.to_owned(),
            });
            target.clone_into(&mut model);
        }
        let grouped = self.view.routing_group(&model, team_id);
        if grouped.is_none()
            && let Some(found) =
                self.early_resolve(&model, team_id, request.authority.proxy_admin)?
        {
            let kept = self.reserved(&model, found, team_id)?;
            return matched(model, kept, aliases);
        }
        self.named_pool(model, grouped, team_id, aliases)
    }

    // The group or name lookup, retried by litellm model, then screened; the flag says a non-empty source was emptied by the screens.
    fn first_pass(
        &self,
        model: &str,
        grouped: Option<Vec<DeploymentSpec>>,
        team_id: Option<&str>,
    ) -> Result<(Vec<DeploymentSpec>, bool), CatalogLookupError> {
        let mut source = grouped.unwrap_or_else(|| all_deployments(self.catalog, model, team_id));
        if source.is_empty() {
            source = self.by_litellm_model(model);
        }
        let had_source = !source.is_empty();
        let screened = self.reserved(
            model,
            self.view.filter_access_groups(model, source, team_id),
            team_id,
        )?;
        let emptied = had_source && screened.is_empty();
        Ok((screened, emptied))
    }

    // The model-name arm: the group or name lookup, the litellm-model retry, the default fallback and the model_alias_map.
    fn named_pool(
        &self,
        mut model: String,
        grouped: Option<Vec<DeploymentSpec>>,
        team_id: Option<&str>,
        mut aliases: Vec<AliasStep>,
    ) -> Result<CatalogMatch, CatalogLookupError> {
        let (mut healthy, access_emptied) = self.first_pass(&model, grouped, team_id)?;
        if healthy.is_empty() {
            if !access_emptied && let Some(fallback) = self.view.default_fallback() {
                model = fallback;
                healthy = self.reserved(
                    &model,
                    self.view.filter_access_groups(
                        &model,
                        all_deployments(self.catalog, &model, team_id),
                        team_id,
                    ),
                    team_id,
                )?;
            }
            if healthy.is_empty() {
                return Err(CatalogLookupError::NoHealthyDeployments { model });
            }
        }
        if let Some(mapped) = self.model_alias_map.get(&model) {
            aliases.push(AliasStep {
                source: AliasSource::ModelAliasMap,
                from: model.clone(),
                to: mapped.clone(),
            });
            model.clone_from(mapped);
        }
        matched(model, healthy, aliases)
    }
}
