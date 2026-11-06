use betterlite_router::simple_shuffle::Candidate;
use betterlite_router::strategy_dispatch::groups::RoutingGroups;
use betterlite_router::strategy_dispatch::override_resolution::{
    bind_override_selector, override_selector, request_override,
};
use betterlite_router::strategy_dispatch::selection::{Selection, select};
use betterlite_router::strategy_dispatch::selector::{Selector, SelectorState};
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use betterlite_router::strategy_dispatch::strategy::RoutingStrategy;
use betterlite_router::strategy_dispatch::types::{
    OverrideDecision, RequestBindings, RoutingRequest, StrategyOverride,
};

use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;
use crate::router_probes::single_candidate::Deployment;

use super::{Function, Input, Output, State};

fn strategy_field(field: Option<&str>) -> Result<RoutingStrategy, OperationError> {
    let text = field.ok_or_else(|| OperationError::InvalidRequest {
        message: "the strategy_dispatch scenario needs a strategy".to_owned(),
    })?;
    RoutingStrategy::parse(text).map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

fn start(input: &Input) -> Result<StrategyDispatch, OperationError> {
    let strategy = strategy_field(input.strategy.as_deref())?;
    StrategyDispatch::new(strategy, None).map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

fn names(selectors: &[Selector]) -> Vec<String> {
    let mut names = Vec::with_capacity(selectors.len());
    for selector in selectors {
        names.push(selector.strategy.wire_name().to_owned());
    }
    names
}

fn snapshot(state: &SelectorState) -> State {
    State {
        selector: state
            .selector
            .map(|selector| selector.strategy.wire_name().to_owned()),
        callbacks: names(&state.callbacks),
        input_callbacks: names(&state.input_callbacks),
    }
}

fn candidates(deployments: &[Deployment]) -> Vec<Candidate> {
    deployments
        .iter()
        .map(|deployment| Candidate {
            weight: deployment.weight,
            rpm: deployment.rpm,
            tpm: deployment.tpm,
        })
        .collect()
}

fn decision_fields(decision: &OverrideDecision) -> (Option<String>, bool, Vec<String>) {
    match decision {
        OverrideDecision::NoOverride => (None, false, Vec::new()),
        OverrideDecision::Honored(honored) => {
            (Some(honored.wire_name().to_owned()), false, Vec::new())
        }
        OverrideDecision::Ignored { supported, .. } => (
            None,
            true,
            supported
                .iter()
                .map(|strategy| strategy.wire_name().to_owned())
                .collect(),
        ),
    }
}

fn override_selector_fields(
    dispatch: &mut StrategyDispatch,
    honored: RoutingStrategy,
) -> (Option<String>, bool, bool, bool) {
    let first = override_selector(dispatch, honored);
    let second = override_selector(dispatch, honored);
    let selector_is_default = first.is_default && first.selector.is_some();
    let selector_cached = first.selector == second.selector;
    let selector = first.selector.map(|_| honored.wire_name().to_owned());
    let mut bindings = RequestBindings::default();
    let bound = bind_override_selector(
        dispatch,
        honored,
        first.selector,
        &RoutingGroups::default(),
        Some(&mut bindings),
    );
    (selector, selector_is_default, selector_cached, bound)
}

pub(super) fn validate(input: &Input) -> Result<Output, OperationError> {
    let value = input
        .value
        .as_deref()
        .ok_or_else(|| OperationError::InvalidRequest {
            message: "validate needs the value to check".to_owned(),
        })?;
    Ok(Output::Validate {
        valid: RoutingStrategy::parse(value).is_ok(),
    })
}

pub(super) fn reinit(input: &Input) -> Result<Output, OperationError> {
    let mut dispatch = start(input)?;
    let next = strategy_field(input.next_strategy.as_deref())?;
    let before = dispatch.state();
    let cached = override_selector(&mut dispatch, RoutingStrategy::CostBasedRouting);
    let again = override_selector(&mut dispatch, RoutingStrategy::CostBasedRouting);
    let override_cached_before = cached.selector.is_some() && cached.selector == again.selector;
    dispatch
        .init(next, None)
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    let after = dispatch.state();
    let old = before.selector.map(|selector| selector.id);
    let old_registered_after = before.selector.is_some()
        && after
            .callbacks
            .iter()
            .any(|selector| Some(selector.id) == old);
    let new = after.selector.map(|selector| selector.id);
    let rebuilt = override_selector(&mut dispatch, RoutingStrategy::CostBasedRouting);
    Ok(Output::Reinit {
        before: snapshot(&before),
        after: snapshot(&after),
        old_registered_after,
        selector_replaced: old != new,
        override_cached_before,
        override_rebuilt_after: cached.selector != rebuilt.selector,
    })
}

pub(super) fn scenario(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<Output, OperationError> {
    let mut dispatch = start(input)?;
    match input.function {
        Function::Override => Ok(override_output(&mut dispatch, input)),
        Function::Dispatch => dispatch_output(&mut dispatch, input, random),
        Function::Validate | Function::Reinit => Err(OperationError::InvalidRequest {
            message: "validate and reinit do not run a selection scenario".to_owned(),
        }),
    }
}

fn override_output(dispatch: &mut StrategyDispatch, input: &Input) -> Output {
    let raw = input
        .routing_strategy
        .as_deref()
        .map_or(StrategyOverride::Absent, StrategyOverride::Text);
    let decision = request_override(raw);
    let (override_strategy, ignored, supported) = decision_fields(&decision);
    let (selector, selector_is_default, selector_cached, bound) = match decision {
        OverrideDecision::Honored(honored) => override_selector_fields(dispatch, honored),
        OverrideDecision::NoOverride | OverrideDecision::Ignored { .. } => {
            (None, false, true, false)
        }
    };
    Output::Override {
        override_strategy,
        ignored,
        supported,
        selector,
        selector_is_default,
        selector_cached,
        bound,
    }
}

fn simple_shuffle_index(selection: Selection) -> Result<usize, OperationError> {
    match selection {
        Selection::SimpleShuffle { index } => Ok(index),
        Selection::Selector { .. } | Selection::NoSelector => Err(OperationError::InvalidRequest {
            message: "the strategy_dispatch probe selects simple-shuffle deployments only"
                .to_owned(),
        }),
    }
}

fn dispatch_output(
    dispatch: &mut StrategyDispatch,
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<Output, OperationError> {
    let model = input
        .model
        .as_deref()
        .ok_or_else(|| OperationError::InvalidRequest {
            message: "dispatch needs the model group to route".to_owned(),
        })?;
    let mut request = RoutingRequest {
        resolved_model: model,
        strategy: StrategyOverride::Absent,
        bindings: None,
    };
    let outcome = select(
        dispatch,
        &mut request,
        &RoutingGroups::default(),
        &candidates(&input.deployments),
        random,
    )
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })?;
    let index = simple_shuffle_index(outcome.selection)?;
    let deployment =
        input
            .deployments
            .get(index)
            .ok_or_else(|| OperationError::InvalidRequest {
                message: "the selected deployment is out of range".to_owned(),
            })?;
    Ok(Output::Dispatch {
        strategy: outcome.resolution.strategy.wire_name().to_owned(),
        selected: deployment.id.clone(),
    })
}
