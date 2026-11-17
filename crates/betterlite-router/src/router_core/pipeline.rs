mod filters;

use betterlite_models::decisions::Json;
use betterlite_models::deployment_filters::get_order_filtered_deployments;
use rand::Rng;

use self::filters::{pre_call_pool, static_pool};
use crate::context::{self, RouteStage};
use crate::deployment_catalog::lookup::CatalogResolver;
use crate::deployment_catalog::types::{AliasStep, Candidates, CatalogMatch, LookupRequest};
use crate::deployment_clients::clients::{CachedClient, ClientDeployment, ClientLookup};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::attempt::AttemptResult;
use crate::router_core::candidates::{CandidateSet, deployment_ids, without_blocked};
use crate::router_core::error::RoutingError;
use crate::router_core::no_deployment::NoDeployment;
use crate::router_core::route::{RouteContext, RouteRequest};
use crate::static_eligibility::filter::{EligibilityInput, EligibilityServices, eligible};
use crate::strategy::{self, Strategy, StrategyContext};
use crate::strategy_dispatch::groups::RoutingScope;
use crate::strategy_dispatch::override_resolution::resolve;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::types::{RequestBindings, RoutingRequest, StrategyOverride};

// The strategy state one route resolves against: B395's dispatch, the request's override and the groups in scope.
pub struct StrategyRoute<'route> {
    pub dispatch: &'route mut StrategyDispatch,
    pub override_strategy: StrategyOverride<'route>,
    pub scope: RoutingScope<'route>,
}

// What one request resolved to, before any filter touched it.
struct ResolvedRoute {
    group: String,
    aliases: Vec<AliasStep>,
    model_ids: Vec<String>,
}

// Router._async_get_healthy_deployments for the group, or the one deployment a specific request names
pub struct HealthyDeployments {
    pub group: String,
    pub healthy: Candidates,
    pub all: Vec<DeploymentSpec>,
    pub aliases: Vec<AliasStep>,
}

pub async fn healthy_deployments(
    resolver: &CatalogResolver<'_>,
    context: &RouteContext<'_>,
) -> Result<HealthyDeployments, RoutingError> {
    let matched = resolve_catalog(resolver, context.request)?;
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

// Router.async_get_available_deployment: resolve, narrow and select in one pass.
pub async fn route<R>(
    resolver: &CatalogResolver<'_>,
    context: &RouteContext<'_>,
    routing: StrategyRoute<'_>,
    random: &mut R,
) -> Result<AttemptResult, RoutingError>
where
    R: Rng + Send,
{
    let matched = resolve_catalog(resolver, context.request)?;
    let resolved = ResolvedRoute {
        model_ids: deployment_ids(matched.candidates.deployments()),
        group: matched.group,
        aliases: matched.aliases,
    };
    match matched.candidates {
        Candidates::Single(deployment) => single_route(context, resolved, deployment),
        Candidates::Pool(pool) => pool_route(context, routing, resolved, pool, random).await,
    }
}

// Router._common_checks_available_deployment, whose typed answer B302 already is
fn resolve_catalog(
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

// A named deployment comes back once the static gate accepted it, with no cooldown filter and no strategy.
fn single_route(
    context: &RouteContext<'_>,
    resolved: ResolvedRoute,
    deployment: Box<DeploymentSpec>,
) -> Result<AttemptResult, RoutingError> {
    let filtered = eligible(
        EligibilityInput {
            model: &resolved.group,
            candidates: Candidates::Single(deployment),
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
    routing: StrategyRoute<'_>,
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
    let pick = select_index(context, &resolved.group, routing, &set, random).await?;
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
    let windowed = context::run(RouteStage::AccessWindows, candidates, context).await?;
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

// B395 resolves which strategy applies, and that strategy's own module picks the deployment.
async fn select_index<R>(
    context: &RouteContext<'_>,
    group: &str,
    routing: StrategyRoute<'_>,
    set: &CandidateSet,
    random: &mut R,
) -> Result<Option<(Strategy, usize)>, RoutingError>
where
    R: Rng + Send,
{
    let mut bindings = RequestBindings::default();
    let mut request = RoutingRequest {
        resolved_model: group,
        strategy: routing.override_strategy,
        bindings: Some(&mut bindings),
    };
    let resolution = resolve(routing.dispatch, &mut request, &routing.scope);
    let Some(strategy) = Strategy::of(resolution.strategy) else {
        return Ok(None);
    };
    let index = strategy::select(
        strategy,
        StrategyContext {
            deployments: &set.deployments,
            route: context,
            dispatch: routing.dispatch,
            resolution: &resolution,
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
    }
}

// Router._get_client for the selected deployment: the client its provider call runs with
fn acquire_client(
    context: &RouteContext<'_>,
    group: &str,
    deployment: &DeploymentSpec,
) -> Option<CachedClient> {
    let access = context.clients?;
    let model_id = deployment.id()?;
    let api_base = match deployment.litellm_params.get("api_base") {
        Some(Json::Text(base)) => base.as_str(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            ""
        }
    };
    Some(access.clients.openai_client(
        access.kind,
        &ClientLookup {
            model_id,
            stream: access.stream,
            api_key: None,
        },
        &ClientDeployment {
            model_group: group,
            api_base,
        },
    ))
}
