use std::collections::BTreeSet;
use std::convert::Infallible;
use std::future::Future;

use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;
use betterlite_models::deployment_filters::OrderedDeployment as _;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_catalog_keys::ModelCostKeys;

use crate::catalog_queries::CatalogQueries;
use crate::catalog_queries::error::CatalogQueryError;
use crate::catalog_queries::messages::{
    MessagesRequest, RouterFallbacks, a_higher_order_level_remains, content_policy_available,
};
use crate::deployment_catalog::scope::litellm_model;
use crate::deployment_model_info::text;
use crate::fallback_plans::groups::{
    fallback_lookup_groups, fallbacks_disabled_for_request, get_pre_routing_selection,
};
use crate::fallback_plans::plan::resolve_chain;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_info::sources::{DeploymentInfo, GroupSources, ProviderResolver};

impl CatalogQueries<'_> {
    // Router.is_recognized_model: a deployment name, id or alias, or a callable routing group.
    #[must_use]
    pub fn is_recognized_model(&self, model: &str) -> bool {
        self.lookup.catalog.model_names.contains(model)
            || self.lookup.catalog.id_index.contains_key(model)
            || self.lookup.model_group_alias.contains_key(model)
            || self.lookup.routing_groups.callable_group(model).is_some()
    }

    // Router.map_team_model: the first row the team owns gives the caller's name, or names its deployment.
    pub fn map_team_model(
        &self,
        team_model_name: Option<&str>,
        team_id: &str,
    ) -> Result<Option<String>, CatalogQueryError> {
        let rows = self.lookup.get_model_list(team_model_name, Some(team_id))?;
        let Some(row) = rows
            .iter()
            .find(|found| text(&found.model_info, "team_id") == Some(team_id))
        else {
            return Ok(None);
        };
        if let Some(name) = team_model_name {
            return Ok(Some(name.to_owned()));
        }
        let public =
            text(&row.model_info, "team_public_model_name").filter(|name| !name.is_empty());
        Ok(Some(
            public.map_or_else(|| row.model_name.clone(), str::to_owned),
        ))
    }

    // Router.resolved_litellm_models: the provider model of every row the name reaches, in listing order.
    pub fn resolved_litellm_models(
        &self,
        model_name: &str,
        team_id: Option<&str>,
    ) -> Result<Vec<String>, CatalogQueryError> {
        let mut models = Vec::new();
        for row in self.lookup.get_model_list(Some(model_name), team_id)? {
            let provider_model = litellm_model(&row);
            if !provider_model.is_empty() {
                models.push(provider_model.to_owned());
            }
        }
        Ok(models)
    }

    // Router._is_model_access_group_for_wildcard_route: a wildcard group holds a model the pattern router matches.
    pub fn is_wildcard_access_group(&self, group: &str) -> Result<bool, CatalogQueryError> {
        let declared =
            self.access_groups
                .get_model_access_groups(&self.lookup, None, Some(group), None)?;
        let Some(models) = declared.get(group) else {
            return Ok(false);
        };
        Ok(models.iter().any(|listed| {
            self.lookup
                .catalog
                .pattern_router
                .route(listed, None)
                .is_some()
        }))
    }

    // Router._anthropic_messages_order_levels: the distinct order levels the fallback dispatcher sees.
    pub fn messages_order_levels(
        &self,
        model_group: &str,
        request: &MessagesRequest<'_>,
    ) -> Result<Vec<i64>, CatalogQueryError> {
        let order_group = get_pre_routing_selection(request.metadata).unwrap_or(model_group);
        let mut levels = BTreeSet::new();
        for deployment in self
            .lookup
            .get_model_list(Some(order_group), request.team_id)?
        {
            if let Some(level) = deployment.order() {
                levels.insert(level);
            }
        }
        Ok(levels.into_iter().collect())
    }

    // Router._anthropic_messages_stream_can_fall_back: whether a mid-stream error could still fall back.
    pub fn messages_can_fall_back(
        &self,
        model_group: &str,
        request: &MessagesRequest<'_>,
        router: &RouterFallbacks<'_>,
    ) -> Result<bool, CatalogQueryError> {
        if fallbacks_disabled_for_request(request.metadata, request.disable_fallbacks) {
            return Ok(false);
        }
        if router.weighted_failover.enabled {
            return Ok(true);
        }
        if a_higher_order_level_remains(self, model_group, request)? {
            return Ok(true);
        }
        if content_policy_available(model_group, request, router) {
            return Ok(true);
        }
        let fallbacks = request
            .fallbacks
            .unwrap_or(router.settings.fallbacks.as_slice());
        if fallbacks.is_empty() {
            return Ok(false);
        }
        // _check_non_standard_fallback_format: bare targets are the client's own fallback format
        if fallbacks
            .iter()
            .all(|entry| matches!(entry, FallbackChainEntry::Target(_)))
        {
            return Ok(true);
        }
        let lookup_groups = fallback_lookup_groups(request.metadata, Some(model_group));
        Ok(resolve_chain(
            fallbacks,
            &lookup_groups,
            &router.provider_names,
            request.attempted_targets,
        )
        .is_some_and(|found| found.has_unattempted))
    }

    // Router.cached_model_group_info: the group info B10047 caches for the name, unchanged.
    pub fn cached_model_group_info<'call, D, R, P>(
        &'call self,
        sources: &'call GroupSources<'call, D, R, P>,
        model_group: &'call str,
        keys: &'call mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<ModelGroupInfo>, GroupInfoError<R::Error>>> + Send + 'call
    where
        D: DeploymentInfo + Sync,
        R: ProviderResolver + Sync,
        P: SupportedParameterSource<Error = Infallible>,
    {
        self.group_info
            .cached_get_model_group_info(&self.lookup, sources, model_group, keys)
    }
}
