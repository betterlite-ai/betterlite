use betterlite_config::sections::fallback_plans;
use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;
use betterlite_config::sections::weighted_failover;

use crate::catalog_queries::CatalogQueries;
use crate::catalog_queries::error::CatalogQueryError;
use crate::fallback_plans::attempts::AttemptedTargets;
use crate::fallback_plans::chains::ProviderNames;
use crate::fallback_plans::groups::{RequestMetadata, fallback_lookup_groups};
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

// The order test: a level above the attempt's own target can still take the stream over.
pub(crate) fn a_higher_order_level_remains(
    queries: &CatalogQueries<'_>,
    model_group: &str,
    request: &MessagesRequest<'_>,
) -> Result<bool, CatalogQueryError> {
    let levels = queries.messages_order_levels(model_group, request)?;
    if levels.len() < 2 {
        return Ok(false);
    }
    let Some(first_level) = levels.first().copied() else {
        return Ok(false);
    };
    let skip_up_to = request.target_order.unwrap_or(first_level);
    Ok(levels.iter().any(|level| *level > skip_up_to))
}

// _has_content_policy_fallback: the request list wins; an unset one leaves the router's list.
pub(crate) fn content_policy_available(
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
