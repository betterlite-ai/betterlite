use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use crate::strategy_dispatch::args::SelectorArgs;
use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroupRoute, RoutingGroups};
use crate::strategy_dispatch::override_resolution::{override_pre_call_target, override_selector};
use crate::strategy_dispatch::selection::{Selection, SelectorCall, select, selector_call};
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::ArgsUpdate;

use super::{SEED, candidates, latency_arguments, new_dispatch, no_shadowing, request_for, scope};

#[test]
fn error_fields_are_typed() {
    let error = RoutingStrategy::parse("priority").unwrap_err();
    let StrategyDispatchError::InvalidRoutingStrategy { value, valid } = error else {
        panic!("an unknown strategy is an invalid routing strategy");
    };
    assert_eq!(value, "priority");
    assert!(valid.contains("'provider-budget-routing'"));
    let message = StrategyDispatchError::InvalidRoutingStrategy { value, valid }.to_string();
    assert!(message.contains("Invalid routing_strategy: 'priority'"));
    assert!(message.contains("Valid options:"));
    let mismatch = StrategyDispatch::new(
        RoutingStrategy::UsageBasedRoutingV2,
        Some(SelectorArgs::Fixed),
    )
    .unwrap_err();
    let StrategyDispatchError::ArgsMismatch {
        strategy,
        expected,
        supplied,
    } = mismatch
    else {
        panic!("fixed arguments do not fit usage-based routing");
    };
    assert_eq!(strategy, RoutingStrategy::UsageBasedRoutingV2);
    assert_eq!(expected, "usage-based arguments");
    assert_eq!(supplied, "no routing arguments");
}

#[test]
fn unregister_retires_by_identity() {
    let mut dispatch = new_dispatch(RoutingStrategy::LeastBusy);
    let selector = dispatch.state().selector.unwrap();
    let unknown = SelectorId(99);
    let error = dispatch.unregister(&[unknown]).unwrap_err();
    let StrategyDispatchError::UnknownSelector { id } = error else {
        panic!("an unknown id must be refused");
    };
    assert_eq!(id, unknown);
    assert!(
        StrategyDispatchError::UnknownSelector { id }
            .to_string()
            .contains("99")
    );
    let state = dispatch.unregister(&[selector.id]).unwrap();
    assert!(state.callbacks.is_empty());
    assert!(state.input_callbacks.is_empty());
    assert!(state.selector.unwrap().retired);
    let again = dispatch.unregister(&[selector.id]).unwrap();
    assert!(again.callbacks.is_empty());
}

#[rstest]
#[case::least_busy(RoutingStrategy::LeastBusy, Some(SelectorCall::LeastBusy))]
#[case::usage_v1(RoutingStrategy::UsageBasedRouting, Some(SelectorCall::UsageBasedV1))]
#[case::usage_v2(RoutingStrategy::UsageBasedRoutingV2, Some(SelectorCall::UsageBasedV2))]
#[case::cost(RoutingStrategy::CostBasedRouting, Some(SelectorCall::CostBased))]
#[case::latency(RoutingStrategy::LatencyBasedRouting, Some(SelectorCall::LatencyBased))]
#[case::simple_shuffle(RoutingStrategy::SimpleShuffle, None)]
#[case::lar1(RoutingStrategy::Lar1, None)]
#[case::provider_budget(RoutingStrategy::ProviderBudgetRouting, None)]
fn selector_call_for_a_present_selector(
    #[case] strategy: RoutingStrategy,
    #[case] expected: Option<SelectorCall>,
) {
    assert_eq!(selector_call(strategy, Some(SelectorId(0))), expected);
}

#[test]
fn selector_call_needs_a_selector() {
    assert_eq!(selector_call(RoutingStrategy::LeastBusy, None), None);
    assert_eq!(
        selector_call(RoutingStrategy::UsageBasedRoutingV2, None),
        None
    );
}

#[test]
fn single_candidate_selection() {
    let mut dispatch = new_dispatch(RoutingStrategy::SimpleShuffle);
    let mut random = StdRng::seed_from_u64(SEED);
    let outcome = select(
        &mut dispatch,
        &mut request_for("gpt"),
        &scope(&RoutingGroups::default(), &no_shadowing()),
        &candidates(),
        &mut random,
    )
    .unwrap();
    assert_eq!(outcome.selection, Selection::SimpleShuffle { index: 0 });
    let error = select(
        &mut dispatch,
        &mut request_for("gpt"),
        &scope(&RoutingGroups::default(), &no_shadowing()),
        &[],
        &mut random,
    )
    .unwrap_err();
    assert!(matches!(error, StrategyDispatchError::Shuffle { .. }));
}

#[test]
fn override_selector_without_a_selector_caches_none() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let overridden = override_selector(&mut dispatch, RoutingStrategy::SimpleShuffle);
    assert_eq!(overridden.selector, None);
    assert!(!overridden.is_default);
    assert_eq!(
        dispatch
            .override_selectors
            .get(&RoutingStrategy::SimpleShuffle),
        Some(&None)
    );
    assert_eq!(
        override_pre_call_target(&dispatch, RoutingStrategy::SimpleShuffle, None),
        None
    );
}

#[test]
fn duplicate_groups_are_refused() {
    let duplicate = RoutingGroups::new(vec![
        RoutingGroupRoute {
            name: "a".to_owned(),
            models: vec!["m".to_owned()],
            strategy: GroupStrategy::Routing(RoutingStrategy::LeastBusy),
            selector: None,
        },
        RoutingGroupRoute {
            name: "a".to_owned(),
            models: vec!["n".to_owned()],
            strategy: GroupStrategy::Routing(RoutingStrategy::LeastBusy),
            selector: None,
        },
    ]);
    assert!(matches!(
        duplicate,
        Err(StrategyDispatchError::DuplicateGroup { .. })
    ));
    let shared = RoutingGroups::new(vec![
        RoutingGroupRoute {
            name: "a".to_owned(),
            models: vec!["m".to_owned()],
            strategy: GroupStrategy::Routing(RoutingStrategy::LeastBusy),
            selector: None,
        },
        RoutingGroupRoute {
            name: "b".to_owned(),
            models: vec!["m".to_owned()],
            strategy: GroupStrategy::Routing(RoutingStrategy::CostBasedRouting),
            selector: None,
        },
    ]);
    assert!(matches!(
        shared,
        Err(StrategyDispatchError::DuplicateGroupModel { .. })
    ));
}

#[test]
fn init_clears_override_cache() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let cached = override_selector(&mut dispatch, RoutingStrategy::CostBasedRouting);
    let again = override_selector(&mut dispatch, RoutingStrategy::CostBasedRouting);
    assert_eq!(cached.selector, again.selector);
    let state = dispatch.init(RoutingStrategy::LeastBusy, None).unwrap();
    assert_eq!(
        state.selector.map(|selector| selector.strategy),
        Some(RoutingStrategy::LeastBusy)
    );
    let rebuilt = override_selector(&mut dispatch, RoutingStrategy::CostBasedRouting);
    assert_ne!(rebuilt.selector, cached.selector);
    assert!(dispatch.selectors[cached.selector.unwrap().0].retired);
}

#[test]
fn registry_edges_refuse_unknown_ids() {
    let mut dispatch = new_dispatch(RoutingStrategy::LeastBusy);
    assert!(matches!(
        dispatch.register_selector(SelectorId(7)),
        Err(StrategyDispatchError::UnknownSelector { .. })
    ));
    assert!(matches!(
        dispatch.unregister(&[SelectorId(1)]),
        Err(StrategyDispatchError::UnknownSelector { .. })
    ));
    let id = dispatch.state().selector.unwrap().id;
    let state = dispatch.unregister(&[id]).unwrap();
    assert!(state.callbacks.is_empty());
}

#[rstest]
#[case::zero(0.0_f64)]
#[case::above_one(5.0_f64)]
#[case::negative(-0.5_f64)]
#[case::nan(f64::NAN)]
fn init_rejects_ttft_percentile_outside_unit_interval(#[case] percentile: f64) {
    let error = StrategyDispatch::new(
        RoutingStrategy::LatencyBasedRouting,
        Some(latency_arguments(Some(percentile))),
    )
    .unwrap_err();
    let StrategyDispatchError::InvalidTtftPercentile { value } = error else {
        panic!("an out-of-range ttft_percentile must be refused");
    };
    assert_eq!(value.to_bits(), percentile.to_bits());
}

#[rstest]
#[case::zero(0.0_f64)]
#[case::above_one(2.5_f64)]
#[case::nan(f64::NAN)]
fn updated_args_reject_ttft_percentile_and_keep_the_previous_selector(#[case] percentile: f64) {
    let mut dispatch = new_dispatch(RoutingStrategy::LatencyBasedRouting);
    let before = dispatch.state();
    let update = dispatch.apply_updated_args(Some(latency_arguments(Some(percentile))));
    let ArgsUpdate::KeptPrevious { error } = update else {
        panic!("invalid latency arguments must keep the previous selector");
    };
    let StrategyDispatchError::InvalidTtftPercentile { value } = error else {
        panic!("an out-of-range ttft_percentile must be reported");
    };
    assert_eq!(value.to_bits(), percentile.to_bits());
    assert_eq!(dispatch.state(), before);
}
