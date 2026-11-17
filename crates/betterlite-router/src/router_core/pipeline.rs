mod clients;
mod filters;

use betterlite_models::deployment_filters::get_order_filtered_deployments;
use rand::Rng;

use self::clients::acquire_client;
use self::filters::{pre_call_pool, static_pool};
use crate::context::{self, RouteStage};
use crate::deployment_catalog::lookup::CatalogResolver;
use crate::deployment_catalog::types::{AliasStep, Candidates, CatalogMatch, LookupRequest};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::attempt::AttemptResult;
use crate::router_core::candidates::{CandidateSet, deployment_ids, without_blocked};
use crate::router_core::error::RoutingError;
use crate::router_core::no_deployment::NoDeployment;
use crate::router_core::route::{RouteContext, RouteRequest};
use crate::static_eligibility::filter::{EligibilityInput, EligibilityServices, eligible};
use crate::strategy::{self, Strategy, StrategyContext};
use crate::strategy_dispatch::groups::RoutingScope;
use crate::strategy_dispatch::override_resolution::{override_pre_call_target, resolve};
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::types::{
    RequestBindings, Resolution, RoutingRequest, StrategyOverride,
};

// The strategy state one route resolves against: B395's dispatch, the request's override and the groups in scope.
pub struct StrategyRoute<'route> {
    pub dispatch: &'route mut StrategyDispatch,
    pub override_strategy: StrategyOverride<'route>,
    pub scope: RoutingScope<'route>,
}

// What one request resolved to, before any filter touched it: the catalog's match and B395's routing decision.
struct ResolvedRoute {
    group: String,
    aliases: Vec<AliasStep>,
    model_ids: Vec<String>,
    resolution: Resolution,
    override_pre_call: Option<SelectorId>,
    bindings: RequestBindings,
}

// Router._async_get_healthy_deployments for the group, or the one deployment a specific request names
#[derive(Debug)]
pub struct HealthyDeployments {
    pub group: String,
    pub healthy: Candidates,
    pub all: Vec<DeploymentSpec>,
    pub aliases: Vec<AliasStep>,
}

pub async fn healthy_deployments(
    context: &RouteContext<'_>,
    matched: CatalogMatch,
) -> Result<HealthyDeployments, RoutingError> {
    let all = matched.candidates.deployments().to_vec();
    if matches!(matched.candidates, Candidates::Single(_)) {
        return Ok(HealthyDeployments {
            group: matched.group,
            healthy: matched.candidates,
            all,
            aliases: matched.aliases,
        });
    }
    let pool = CandidateSet {
        deployments: all.clone(),
    };
    let set = context::run(RouteStage::CooldownState, pool, context).await?;
    Ok(HealthyDeployments {
        group: matched.group,
        healthy: Candidates::Pool(without_blocked(set.deployments)),
        all,
        aliases: matched.aliases,
    })
}

// The catalog's answer and B395's decision, resolved in one synchronous pass so no view crosses an await.
pub struct Resolved {
    route: ResolvedRoute,
    candidates: Candidates,
}

// Router._common_checks_available_deployment and the routing decision that follows it.
pub fn prepare(
    resolver: &CatalogResolver<'_>,
    context: &RouteContext<'_>,
    routing: &mut StrategyRoute<'_>,
) -> Result<Resolved, RoutingError> {
    let matched = resolve_catalog(resolver, context.request)?;
    // LiteLLM resolves the routing context before it looks at the deployments, and every arm keeps that decision.
    let mut bindings = RequestBindings::default();
    let mut routing_request = RoutingRequest {
        resolved_model: &matched.group,
        strategy: routing.override_strategy,
        bindings: Some(&mut bindings),
    };
    let resolution = resolve(routing.dispatch, &mut routing_request, &routing.scope);
    let state = ResolvedRoute {
        model_ids: deployment_ids(matched.candidates.deployments()),
        group: matched.group,
        aliases: matched.aliases,
        override_pre_call: override_pre_call_target(
            routing.dispatch,
            resolution.strategy,
            resolution.selector,
        ),
        resolution,
        bindings,
    };
    Ok(Resolved {
        route: state,
        candidates: matched.candidates,
    })
}

// Router.async_get_available_deployment over what prepare resolved: narrow the pool, then select.
pub async fn route<R>(
    context: &RouteContext<'_>,
    dispatch: &mut StrategyDispatch,
    resolved: Resolved,
    random: &mut R,
) -> Result<AttemptResult, RoutingError>
where
    R: Rng + Send,
{
    let Resolved { route, candidates } = resolved;
    match candidates {
        Candidates::Single(deployment) => single_route(context, route, *deployment).await,
        Candidates::Pool(pool) => pool_route(context, dispatch, route, pool, random).await,
    }
}

// Router._common_checks_available_deployment, whose typed answer B302 already is
pub fn resolve_catalog(
    resolver: &CatalogResolver<'_>,
    request: &RouteRequest<'_>,
) -> Result<CatalogMatch, RoutingError> {
    let lookup = LookupRequest {
        model: request.model,
        specific_deployment: request.specific_deployment,
        authority: request.authority,
    };
    Ok(resolver.resolve(&lookup)?)
}

// A named deployment comes back once the generalization rules and the static gate accepted it, with no strategy.
async fn single_route(
    context: &RouteContext<'_>,
    resolved: ResolvedRoute,
    deployment: DeploymentSpec,
) -> Result<AttemptResult, RoutingError> {
    // get_model_info fills every registered deployment from the rules, a named one included
    let candidates = context::run(
        RouteStage::GeneralizationIntegration,
        CandidateSet {
            deployments: vec![deployment],
        },
        context,
    )
    .await?;
    let Some(filled) = candidates.deployments.into_iter().next() else {
        return Err(no_deployment_error(context, &resolved));
    };
    let filtered = eligible(
        EligibilityInput {
            model: &resolved.group,
            candidates: Candidates::Single(Box::new(filled)),
            facts: context.request.facts,
            environment: context.request.environment,
            enable_pre_call_checks: false,
            drop_params: context.policy.drop_params,
        },
        &EligibilityServices {
            limits: context.limits,
            params: context.params,
        },
    )?;
    let Some(selected) = filtered.deployments.into_iter().next() else {
        return Err(no_deployment_error(context, &resolved));
    };
    Ok(attempt(context, resolved, selected, None))
}

async fn pool_route<R>(
    context: &RouteContext<'_>,
    dispatch: &mut StrategyDispatch,
    resolved: ResolvedRoute,
    pool: Vec<DeploymentSpec>,
    random: &mut R,
) -> Result<AttemptResult, RoutingError>
where
    R: Rng + Send,
{
    let set = narrowed_pool(context, &resolved.group, pool).await?;
    if set.deployments.is_empty() {
        return Err(no_deployment_error(context, &resolved));
    }
    // an encrypted-content affinity pin names one deployment, so nothing is left for a strategy to pick
    if context.request.affinity_pinned && set.deployments.len() == 1 {
        let Some(deployment) = set.deployments.into_iter().next() else {
            return Err(no_deployment_error(context, &resolved));
        };
        return Ok(attempt(context, resolved, deployment, None));
    }
    let pick = select_index(context, dispatch, &resolved.resolution, &set, random).await?;
    let Some((strategy, index)) = pick else {
        return Err(no_deployment_error(context, &resolved));
    };
    // a strategy picks an index of the set it was given; anything else is the same no-deployment answer
    let Some(deployment) = set.deployments.into_iter().nth(index) else {
        return Err(no_deployment_error(context, &resolved));
    };
    Ok(attempt(context, resolved, deployment, Some(strategy)))
}

// The pool after every filter async_get_healthy_deployments applies before it selects.
async fn narrowed_pool(
    context: &RouteContext<'_>,
    group: &str,
    pool: Vec<DeploymentSpec>,
) -> Result<CandidateSet, RoutingError> {
    let candidates = CandidateSet { deployments: pool };
    let generalized =
        context::run(RouteStage::GeneralizationIntegration, candidates, context).await?;
    let windowed = context::run(RouteStage::AccessWindows, generalized, context).await?;
    let mut set = CandidateSet {
        deployments: static_pool(context, windowed.deployments)?,
    };
    set = context::run(RouteStage::CooldownState, set, context).await?;
    set = CandidateSet {
        deployments: without_blocked(set.deployments),
    };
    let kept = context.services.filter_deployments(set.deployments)?;
    set = CandidateSet {
        deployments: pre_call_pool(context, group, kept)?,
    };
    set = context::run(RouteStage::TagRouting, set, context).await?;
    set = context::run(RouteStage::RoutingPlugins, set, context).await?;
    set = CandidateSet {
        deployments: get_order_filtered_deployments(
            set.deployments,
            context.request.facts.target_order,
        ),
    };
    set = set.excluded(context.request.excluded_deployment_ids);
    let without_skipped = set
        .clone()
        .excluded(context.request.retry_skipped_deployment_ids);
    if !without_skipped.deployments.is_empty() {
        set = without_skipped;
    }
    Ok(set)
}

// B395 resolved which strategy applies; that strategy's own module picks the deployment.
async fn select_index<R>(
    context: &RouteContext<'_>,
    dispatch: &mut StrategyDispatch,
    resolution: &Resolution,
    set: &CandidateSet,
    random: &mut R,
) -> Result<Option<(Strategy, usize)>, RoutingError>
where
    R: Rng + Send,
{
    let Some(strategy) = Strategy::of(resolution.strategy) else {
        return Ok(None);
    };
    let index = strategy::select(
        strategy,
        StrategyContext {
            deployments: &set.deployments,
            route: context,
            dispatch,
            resolution,
        },
        random,
    )
    .await?;
    Ok(Some((strategy, index)))
}

// async_raise_no_deployment_exception, in the fields the proxy and the caller read back
fn no_deployment_error(context: &RouteContext<'_>, resolved: &ResolvedRoute) -> RoutingError {
    RoutingError::NoDeployment(NoDeployment::new(
        resolved.group.clone(),
        context.services.min_cooldown(&resolved.model_ids),
        context.policy.enable_pre_call_checks,
        context.services.cooldown_list(),
        resolved.model_ids.clone(),
    ))
}

fn attempt(
    context: &RouteContext<'_>,
    resolved: ResolvedRoute,
    deployment: DeploymentSpec,
    strategy: Option<Strategy>,
) -> AttemptResult {
    let client = acquire_client(context, &resolved.group, &deployment);
    AttemptResult {
        group: resolved.group,
        deployment,
        aliases: resolved.aliases,
        strategy,
        client,
        override_pre_call: resolved.override_pre_call,
        bindings: resolved.bindings,
    }
}
