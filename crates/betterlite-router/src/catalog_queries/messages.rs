use std::collections::BTreeSet;

use betterlite_config::sections::fallback_plans;
use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;
use betterlite_config::sections::weighted_failover;
use betterlite_models::deployment_filters::OrderedDeployment as _;

use crate::catalog_queries::CatalogQueries;
use crate::catalog_queries::error::CatalogQueryError;
use crate::fallback_plans::attempts::AttemptedTargets;
use crate::fallback_plans::chains::ProviderNames;
use crate::fallback_plans::groups::{
    RequestMetadata, fallback_lookup_groups, fallbacks_disabled_for_request,
    get_pre_routing_selection,
};
use crate::fallback_plans::plan::resolve_chain;

// The request values the Messages fallback check reads, decoded once from one call's kwargs.
pub struct MessagesRequest<'request> {
    pub metadata: &'request RequestMetadata,
    // metadata.user_api_key_team_id, the team the order levels list deployments for
    pub team_id: Option<&'request str>,
    pub target_order: Option<i64>,
    pub fallbacks: Option<&'request [FallbackChainEntry]>,
    pub content_policy_fallbacks: Option<&'request [FallbackChainEntry]>,
    pub disable_fallbacks: bool,
    pub attempted_targets: &'request AttemptedTargets,
}

// The router configuration the check reads, beside the provider tables a fallback key resolves through.
pub struct RouterFallbacks<'router> {
    pub settings: &'router fallback_plans::Settings,
    pub weighted_failover: weighted_failover::Settings,
    pub provider_names: ProviderNames<'router>,
}

impl CatalogQueries<'_> {
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
        if self.a_higher_order_level_remains(model_group, request)? {
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

    // The order test: a level above the attempt's own target can still take the stream over.
    fn a_higher_order_level_remains(
        &self,
        model_group: &str,
        request: &MessagesRequest<'_>,
    ) -> Result<bool, CatalogQueryError> {
        let levels = self.messages_order_levels(model_group, request)?;
        if levels.len() < 2 {
            return Ok(false);
        }
        let Some(first_level) = levels.first().copied() else {
            return Ok(false);
        };
        let skip_up_to = request.target_order.unwrap_or(first_level);
        Ok(levels.iter().any(|level| *level > skip_up_to))
    }
}

// _has_content_policy_fallback: the request list wins; an unset one leaves the router's list.
fn content_policy_available(
    model_group: &str,
    request: &MessagesRequest<'_>,
    router: &RouterFallbacks<'_>,
) -> bool {
    let content_policy = request
        .content_policy_fallbacks
        .unwrap_or(router.settings.content_policy_fallbacks.as_slice());
    let lookup_groups = fallback_lookup_groups(request.metadata, Some(model_group));
    resolve_chain(
        content_policy,
        &lookup_groups,
        &router.provider_names,
        request.attempted_targets,
    )
    .is_some_and(|found| found.has_unattempted)
}
