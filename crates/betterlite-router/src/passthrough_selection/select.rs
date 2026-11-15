use rand::Rng;

use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_catalog::lookup::CatalogResolver;
use crate::deployment_catalog::types::{Candidates, CatalogMatch, LookupRequest};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::passthrough_selection::error::PassthroughSelectionError;
use crate::passthrough_selection::params::{metrics, pass_through_flag};
use crate::passthrough_selection::selector::SelectorCaller;
use crate::passthrough_selection::types::{Selected, SelectionRequest, SelectionServices};
use crate::simple_shuffle::Candidate;
use crate::static_eligibility::filter::{Cause, EligibilityInput, eligible};
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::groups::RoutingScope;
use crate::strategy_dispatch::override_resolution::{override_pre_call_target, resolve};
use crate::strategy_dispatch::selection::{self, Selection, SelectionOutcome};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::types::RoutingRequest;

// The pass-through deployments that survived eligibility, with the shuffle metrics read from their params.
struct Pool {
    routing_model: String,
    deployments: Vec<DeploymentSpec>,
    candidates: Vec<Candidate>,
}

// One lookup: what the routing context resolves the request to, what the catalog matched, and what eligibility left.
struct Resolved {
    routing_model: String,
    group: String,
    single: bool,
    deployments: Vec<DeploymentSpec>,
    causes: Vec<Cause>,
}

// Router._get_model_from_alias: the routing context reads the model_group_alias target, never the model_alias_map.
fn routing_model(request: &SelectionRequest<'_>, catalog: &CatalogResolver<'_>) -> String {
    match catalog.model_group_alias.get(request.model) {
        Some(target) if !target.is_empty() => target.clone(),
        Some(_) | None => request.model.to_owned(),
    }
}

// Router.async_get_available_deployment_for_pass_through as one step: catalog, static filters, pass-through filter, strategy.
pub fn select<R>(
    dispatch: &mut StrategyDispatch,
    picker: &mut dyn SelectorCaller,
    request: &mut SelectionRequest<'_>,
    services: &SelectionServices<'_, '_>,
    random: &mut R,
) -> Result<Selected, PassthroughSelectionError>
where
    R: Rng,
{
    let resolved = look_up(request, services)?;
    let scope = RoutingScope {
        groups: services.groups,
        shadowing: services.shadowing,
    };
    if resolved.single {
        return deployment_id_selection(
            dispatch,
            request,
            &scope,
            &resolved.routing_model,
            resolved.deployments,
        );
    }
    if resolved.deployments.is_empty() {
        // async_get_healthy_deployments raises its no-deployments error before the pass-through filter runs
        return empty_candidates(
            request.specific_deployment,
            resolved.group,
            &resolved.causes,
        );
    }
    let pool = pass_through_pool(request.model, &resolved.routing_model, resolved.deployments)?;
    let outcome = select_outcome(dispatch, request, &scope, &pool, random)?;
    chosen_deployment(dispatch, picker, outcome, &pool, request)
}

// A specific lookup always takes the no-deployments error; only a group the environment gate emptied takes the 400.
fn empty_candidates(
    specific_deployment: bool,
    group: String,
    causes: &[Cause],
) -> Result<Selected, PassthroughSelectionError> {
    let unregistered = !specific_deployment
        && !causes.is_empty()
        && causes
            .iter()
            .all(|cause| matches!(cause, Cause::Environment { .. }));
    if unregistered {
        return Err(PassthroughSelectionError::Catalog(
            CatalogLookupError::NoHealthyDeployments { model: group },
        ));
    }
    Err(PassthroughSelectionError::NoDeployment { model: group })
}

// _common_checks_available_deployment and async_get_healthy_deployments; the flag says an id resolved to one deployment.
fn look_up(
    request: &SelectionRequest<'_>,
    services: &SelectionServices<'_, '_>,
) -> Result<Resolved, PassthroughSelectionError> {
    let routing_model = routing_model(request, services.catalog);
    let CatalogMatch {
        group, candidates, ..
    } = services.catalog.resolve(&LookupRequest {
        model: request.model,
        specific_deployment: request.specific_deployment,
        authority: request.authority,
    })?;
    let single = matches!(candidates, Candidates::Single(_));
    let eligibility = eligible(
        EligibilityInput {
            model: &group,
            candidates,
            facts: request.facts,
            environment: services.environment,
            enable_pre_call_checks: services.enable_pre_call_checks,
            drop_params: services.drop_params,
        },
        services.eligibility,
    )?;
    Ok(Resolved {
        routing_model,
        group,
        single,
        deployments: eligibility.deployments,
        causes: eligibility.causes,
    })
}

// The dict arm: the one deployment its id named, checked for the paused flag and for use_in_pass_through.
fn deployment_id_selection(
    dispatch: &mut StrategyDispatch,
    request: &mut SelectionRequest<'_>,
    scope: &RoutingScope<'_>,
    model: &str,
    deployments: Vec<DeploymentSpec>,
) -> Result<Selected, PassthroughSelectionError> {
    let Some(deployment) = deployments.into_iter().next() else {
        // a deployment its environment never registered is not in the id map, so LiteLLM takes the name arm
        return Err(PassthroughSelectionError::Catalog(
            CatalogLookupError::NoHealthyDeployments {
                model: model.to_owned(),
            },
        ));
    };
    let mut routing = RoutingRequest {
        resolved_model: model,
        strategy: request.strategy,
        bindings: request.bindings.as_deref_mut(),
    };
    let resolution = resolve(dispatch, &mut routing, scope);
    if !pass_through_flag(&deployment)? {
        return Err(PassthroughSelectionError::DeploymentNotConfigured {
            deployment_id: deployment.id().unwrap_or_default().to_owned(),
        });
    }
    let override_pre_call =
        override_pre_call_target(dispatch, resolution.strategy, resolution.selector);
    Ok(Selected {
        deployment,
        resolution,
        override_pre_call,
    })
}

// _filter_pass_through_deployments, then the shuffle metrics simple_shuffle reads from the surviving params.
fn pass_through_pool(
    request_model: &str,
    routing_model: &str,
    deployments: Vec<DeploymentSpec>,
) -> Result<Pool, PassthroughSelectionError> {
    let mut passed = Vec::new();
    for deployment in deployments {
        if pass_through_flag(&deployment)? {
            passed.push(deployment);
        }
    }
    if passed.is_empty() {
        return Err(PassthroughSelectionError::NotConfigured {
            model: request_model.to_owned(),
        });
    }
    let candidates = passed.iter().map(metrics).collect::<Result<Vec<_>, _>>()?;
    Ok(Pool {
        routing_model: routing_model.to_owned(),
        deployments: passed,
        candidates,
    })
}

// The strategy step: one resolution for the request, then a simple-shuffle draw or a selector call.
fn select_outcome<R>(
    dispatch: &mut StrategyDispatch,
    request: &mut SelectionRequest<'_>,
    scope: &RoutingScope<'_>,
    pool: &Pool,
    random: &mut R,
) -> Result<SelectionOutcome, PassthroughSelectionError>
where
    R: Rng,
{
    let mut routing = RoutingRequest {
        resolved_model: &pool.routing_model,
        strategy: request.strategy,
        bindings: request.bindings.as_deref_mut(),
    };
    Ok(selection::select(
        dispatch,
        &mut routing,
        scope,
        &pool.candidates,
        random,
    )?)
}

fn chosen_deployment(
    dispatch: &StrategyDispatch,
    picker: &mut (dyn SelectorCaller + '_),
    outcome: SelectionOutcome,
    pool: &Pool,
    request: &SelectionRequest<'_>,
) -> Result<Selected, PassthroughSelectionError> {
    let chosen = chosen_index(picker, &outcome.selection, &pool.deployments, request.facts)?;
    let Some(deployment) = chosen
        .and_then(|index| pool.deployments.get(index))
        .cloned()
    else {
        // the strategy chose nothing: LiteLLM's no-deployment error names the model the caller requested
        return Err(PassthroughSelectionError::NoDeployment {
            model: request.model.to_owned(),
        });
    };
    let resolution = outcome.resolution;
    let override_pre_call =
        override_pre_call_target(dispatch, resolution.strategy, resolution.selector);
    Ok(Selected {
        deployment,
        resolution,
        override_pre_call,
    })
}

// The selector call goes to the port; a missing one and a pick outside the pool both leave no deployment.
fn chosen_index(
    picker: &mut (dyn SelectorCaller + '_),
    selection: &Selection,
    deployments: &[DeploymentSpec],
    facts: &RequestFacts,
) -> Result<Option<usize>, PassthroughSelectionError> {
    match *selection {
        Selection::SimpleShuffle { index } => Ok(Some(index)),
        Selection::Selector { selector, call } => {
            match picker.pick(call, selector, deployments, facts) {
                Some(index) if index < deployments.len() => Ok(Some(index)),
                Some(index) => Err(PassthroughSelectionError::SelectorOutOfRange {
                    selector,
                    index,
                    available: deployments.len(),
                }),
                None => Ok(None),
            }
        }
        Selection::NoSelector => Ok(None),
    }
}
