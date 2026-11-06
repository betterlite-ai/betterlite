use std::collections::BTreeSet;

use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use crate::strategy_dispatch::args::{SelectorArgs, UsageArgs};
use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::groups::RoutingGroups;
use crate::strategy_dispatch::override_resolution::{
    bind_override_selector, globally_registered_strategies, override_pre_call_target,
    override_selector, request_override, resolve,
};
use crate::strategy_dispatch::selection::{Selection, SelectorCall, read_prefetch_plan, select};
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{
    ArgsUpdate, OverrideDecision, RequestBindings, ResolutionSource, RoutingRequest,
    StrategyOverride,
};

use super::{
    SEED, callback_strategies, candidates, cheap_groups, input_callback_strategies,
    latency_arguments, new_dispatch, no_shadowing, request_for, scope,
};

#[rstest]
#[case::simple_shuffle(RoutingStrategy::SimpleShuffle, None, vec![], vec![])]
#[case::least_busy(RoutingStrategy::LeastBusy, Some(RoutingStrategy::LeastBusy), vec![RoutingStrategy::LeastBusy], vec![RoutingStrategy::LeastBusy])]
#[case::usage_based(RoutingStrategy::UsageBasedRouting, Some(RoutingStrategy::UsageBasedRouting), vec![RoutingStrategy::UsageBasedRouting], vec![])]
#[case::usage_based_v2(RoutingStrategy::UsageBasedRoutingV2, Some(RoutingStrategy::UsageBasedRoutingV2), vec![RoutingStrategy::UsageBasedRoutingV2], vec![])]
#[case::latency(RoutingStrategy::LatencyBasedRouting, Some(RoutingStrategy::LatencyBasedRouting), vec![RoutingStrategy::LatencyBasedRouting], vec![])]
#[case::cost(RoutingStrategy::CostBasedRouting, Some(RoutingStrategy::CostBasedRouting), vec![RoutingStrategy::CostBasedRouting], vec![])]
#[case::lar1(RoutingStrategy::Lar1, None, vec![], vec![])]
#[case::provider_budget(RoutingStrategy::ProviderBudgetRouting, None, vec![], vec![])]
fn init_registers_each_selector(
    #[case] strategy: RoutingStrategy,
    #[case] selector: Option<RoutingStrategy>,
    #[case] callbacks: Vec<RoutingStrategy>,
    #[case] input_callbacks: Vec<RoutingStrategy>,
) {
    let state = new_dispatch(strategy).state();
    assert_eq!(state.selector.map(|selected| selected.strategy), selector);
    assert_eq!(callback_strategies(&state), callbacks);
    assert_eq!(input_callback_strategies(&state), input_callbacks);
}

#[test]
fn cost_based_ignores_arguments() {
    let arguments = SelectorArgs::Usage(UsageArgs { ttl_seconds: 5 });
    let dispatch =
        StrategyDispatch::new(RoutingStrategy::CostBasedRouting, Some(arguments)).unwrap();
    assert_eq!(
        dispatch.state().selector.map(|selector| selector.args),
        Some(SelectorArgs::Fixed)
    );
}

#[test]
fn updated_args_rebuild_keeps_the_rebuilt_selector_registered() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let previous = dispatch.state().selector.unwrap().id;
    let update =
        dispatch.apply_updated_args(Some(SelectorArgs::Usage(UsageArgs { ttl_seconds: 7 })));
    let ArgsUpdate::Rebuilt {
        previous: retired,
        selector,
    } = update
    else {
        panic!("the update must rebuild the selector");
    };
    assert_eq!(retired, previous);
    assert_eq!(
        selector.args,
        SelectorArgs::Usage(UsageArgs { ttl_seconds: 7 })
    );
    assert_eq!(dispatch.state().selector, Some(selector));
    assert_eq!(
        callback_strategies(&dispatch.state()),
        vec![RoutingStrategy::UsageBasedRoutingV2]
    );
    assert!(dispatch.selectors[previous.0].retired);
}

#[test]
fn updated_args_keep_the_previous_selector() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let before = dispatch.state();
    let update = dispatch.apply_updated_args(Some(latency_arguments(None)));
    assert!(matches!(
        update,
        ArgsUpdate::KeptPrevious {
            error: StrategyDispatchError::ArgsMismatch { .. }
        }
    ));
    assert_eq!(
        dispatch.args,
        SelectorArgs::Usage(UsageArgs { ttl_seconds: 60 })
    );
    assert_eq!(dispatch.state(), before);
}

#[test]
fn updated_args_delegate_lar1_or_skip_without_selector() {
    let mut lar1 = new_dispatch(RoutingStrategy::Lar1);
    assert!(matches!(
        lar1.apply_updated_args(None),
        ArgsUpdate::Lar1Delegated
    ));
    let mut simple = new_dispatch(RoutingStrategy::SimpleShuffle);
    assert!(matches!(
        simple.apply_updated_args(None),
        ArgsUpdate::NoSelector
    ));
}

#[test]
fn request_override_honored_binds_and_caches() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let mut bindings = RequestBindings::default();
    let mut request = RoutingRequest {
        resolved_model: "gpt",
        strategy: StrategyOverride::Text("least-busy"),
        bindings: Some(&mut bindings),
    };
    let resolution = resolve(
        &mut dispatch,
        &mut request,
        &scope(&RoutingGroups::default(), &no_shadowing()),
    );
    assert_eq!(resolution.strategy, RoutingStrategy::LeastBusy);
    assert_eq!(resolution.source, ResolutionSource::RequestOverride);
    assert!(resolution.bound);
    assert_eq!(
        bindings.dynamic_callbacks,
        vec![resolution.selector.unwrap()]
    );
    let cached = override_selector(&mut dispatch, RoutingStrategy::LeastBusy);
    assert_eq!(cached.selector, resolution.selector);
    assert!(!cached.is_default);
    assert_eq!(
        override_pre_call_target(&dispatch, RoutingStrategy::LeastBusy, cached.selector),
        cached.selector
    );
}

#[test]
fn request_override_matches_the_default_selector() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let default = dispatch.state().selector.unwrap().id;
    let overridden = override_selector(&mut dispatch, RoutingStrategy::UsageBasedRoutingV2);
    assert_eq!(overridden.selector, Some(default));
    assert!(overridden.is_default);
    let mut bindings = RequestBindings::default();
    let bound = bind_override_selector(
        &dispatch,
        RoutingStrategy::UsageBasedRoutingV2,
        overridden.selector,
        &RoutingGroups::default(),
        Some(&mut bindings),
    );
    assert!(!bound);
    assert!(bindings.dynamic_callbacks.is_empty());
    assert_eq!(
        override_pre_call_target(
            &dispatch,
            RoutingStrategy::UsageBasedRoutingV2,
            overridden.selector
        ),
        None
    );
}

#[rstest]
#[case::lar1(StrategyOverride::Text("lar1"), Some("lar1"))]
#[case::unknown(StrategyOverride::Text("nope"), Some("nope"))]
#[case::non_text(StrategyOverride::NotText, None)]
fn request_override_ignores_unsupported_values(
    #[case] raw: StrategyOverride<'static>,
    #[case] raw_text: Option<&'static str>,
) {
    let OverrideDecision::Ignored {
        raw: value,
        supported,
    } = request_override(raw)
    else {
        panic!("the override must be ignored");
    };
    assert_eq!(value.as_deref(), raw_text);
    let names: Vec<&str> = supported
        .iter()
        .map(|strategy| strategy.wire_name())
        .collect();
    assert_eq!(
        names,
        vec![
            "cost-based-routing",
            "latency-based-routing",
            "least-busy",
            "simple-shuffle",
            "usage-based-routing",
            "usage-based-routing-v2",
        ]
    );
}

#[test]
fn deployed_model_name_shadows_a_group_name() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let groups = cheap_groups();
    let shadowed = BTreeSet::from(["cheap".to_owned()]);
    let resolution = resolve(
        &mut dispatch,
        &mut request_for("cheap"),
        &scope(&groups, &shadowed),
    );
    assert_eq!(resolution.strategy, RoutingStrategy::UsageBasedRoutingV2);
    assert_eq!(resolution.source, ResolutionSource::Default);
    let member_shadowed = BTreeSet::from(["fast".to_owned()]);
    let member = resolve(
        &mut dispatch,
        &mut request_for("fast"),
        &scope(&groups, &member_shadowed),
    );
    assert_eq!(member.strategy, RoutingStrategy::LeastBusy);
    assert_eq!(
        member.source,
        ResolutionSource::RoutingGroup("cheap".to_owned())
    );
}

#[test]
fn group_resolution_follows_group_and_default() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let groups = cheap_groups();
    let shadowing = no_shadowing();
    let callable = resolve(
        &mut dispatch,
        &mut request_for("cheap"),
        &scope(&groups, &shadowing),
    );
    assert_eq!(callable.strategy, RoutingStrategy::LeastBusy);
    assert_eq!(
        callable.source,
        ResolutionSource::RoutingGroup("cheap".to_owned())
    );
    let member = resolve(
        &mut dispatch,
        &mut request_for("fast"),
        &scope(&groups, &shadowing),
    );
    assert_eq!(member.strategy, RoutingStrategy::LeastBusy);
    let priority = resolve(
        &mut dispatch,
        &mut request_for("prio"),
        &scope(&groups, &shadowing),
    );
    assert_eq!(priority.strategy, RoutingStrategy::SimpleShuffle);
    assert_eq!(priority.selector, None);
    let default = resolve(
        &mut dispatch,
        &mut request_for("slow"),
        &scope(&groups, &shadowing),
    );
    assert_eq!(default.strategy, RoutingStrategy::UsageBasedRoutingV2);
    assert_eq!(default.source, ResolutionSource::Default);
}

#[test]
fn read_prefetch_plan_pairs_the_usage_selector() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let deployments = ["one".to_owned(), "two".to_owned()];
    let plan = read_prefetch_plan(
        &mut dispatch,
        &mut request_for("gpt"),
        &scope(&RoutingGroups::default(), &no_shadowing()),
        &deployments,
    )
    .unwrap();
    assert_eq!(
        plan.usage_selector,
        dispatch.state().selector.map(|item| item.id)
    );
    assert_eq!(plan.deployments.len(), 2);
    assert!(
        read_prefetch_plan(
            &mut dispatch,
            &mut request_for("gpt"),
            &scope(&RoutingGroups::default(), &no_shadowing()),
            &Vec::<String>::new(),
        )
        .is_none()
    );
    let mut simple = new_dispatch(RoutingStrategy::SimpleShuffle);
    let simple_plan = read_prefetch_plan(
        &mut simple,
        &mut request_for("gpt"),
        &scope(&RoutingGroups::default(), &no_shadowing()),
        &deployments,
    )
    .unwrap();
    assert_eq!(simple_plan.usage_selector, None);
}

#[test]
fn binding_skips_globally_registered_strategies() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let groups = cheap_groups();
    let mut bindings = RequestBindings::default();
    let mut request = RoutingRequest {
        resolved_model: "gpt",
        strategy: StrategyOverride::Text("least-busy"),
        bindings: Some(&mut bindings),
    };
    let resolution = resolve(
        &mut dispatch,
        &mut request,
        &scope(&groups, &no_shadowing()),
    );
    assert_eq!(resolution.strategy, RoutingStrategy::LeastBusy);
    assert!(!resolution.bound);
    assert!(bindings.dynamic_callbacks.is_empty());
    assert_eq!(
        globally_registered_strategies(&dispatch, &groups),
        BTreeSet::from([
            RoutingStrategy::UsageBasedRoutingV2,
            RoutingStrategy::LeastBusy,
        ])
    );
}

#[test]
fn group_selectors_stay_registered() {
    let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let built = dispatch
        .build_group_selector(RoutingStrategy::LeastBusy, None)
        .unwrap()
        .unwrap();
    assert!(
        dispatch
            .state()
            .callbacks
            .iter()
            .all(|selector| selector.id != built)
    );
    dispatch.register_selector(built).unwrap();
    let state = dispatch.state();
    assert_eq!(
        callback_strategies(&state),
        vec![
            RoutingStrategy::UsageBasedRoutingV2,
            RoutingStrategy::LeastBusy
        ]
    );
    assert_eq!(
        input_callback_strategies(&state),
        vec![RoutingStrategy::LeastBusy]
    );
    // LiteLLM keys callbacks by class, so it would drop this second usage-based-v2 selector
    let sibling = dispatch
        .build_group_selector(RoutingStrategy::UsageBasedRoutingV2, None)
        .unwrap()
        .unwrap();
    dispatch.register_selector(sibling).unwrap();
    let registered = callback_strategies(&dispatch.state())
        .iter()
        .filter(|strategy| **strategy == RoutingStrategy::UsageBasedRoutingV2)
        .count();
    assert_eq!(registered, 2);
}

#[rstest]
#[case::least_busy(RoutingStrategy::LeastBusy, SelectorCall::LeastBusy)]
#[case::usage_v2(RoutingStrategy::UsageBasedRoutingV2, SelectorCall::UsageBasedV2)]
fn selector_strategies_delegate_selection(
    #[case] strategy: RoutingStrategy,
    #[case] call: SelectorCall,
) {
    let mut dispatch = new_dispatch(strategy);
    let expected = dispatch.state().selector.unwrap().id;
    let mut random = StdRng::seed_from_u64(SEED);
    let outcome = select(
        &mut dispatch,
        &mut request_for("gpt"),
        &scope(&RoutingGroups::default(), &no_shadowing()),
        &candidates(),
        &mut random,
    )
    .unwrap();
    assert_eq!(
        outcome.selection,
        Selection::Selector {
            selector: expected,
            call,
        }
    );
}

#[test]
fn lar1_and_provider_budget_have_no_selection() {
    let mut random = StdRng::seed_from_u64(SEED);
    let mut lar1 = new_dispatch(RoutingStrategy::Lar1);
    let outcome = select(
        &mut lar1,
        &mut request_for("gpt"),
        &scope(&RoutingGroups::default(), &no_shadowing()),
        &candidates(),
        &mut random,
    )
    .unwrap();
    assert_eq!(outcome.selection, Selection::NoSelector);
    let mut budget = new_dispatch(RoutingStrategy::ProviderBudgetRouting);
    let budget_outcome = select(
        &mut budget,
        &mut request_for("gpt"),
        &scope(&RoutingGroups::default(), &no_shadowing()),
        &candidates(),
        &mut random,
    )
    .unwrap();
    assert_eq!(budget_outcome.selection, Selection::NoSelector);
}

#[rstest]
#[case::simple_shuffle(RoutingStrategy::SimpleShuffle, "simple-shuffle")]
#[case::lar1(RoutingStrategy::Lar1, "lar1")]
#[case::least_busy(RoutingStrategy::LeastBusy, "least-busy")]
#[case::latency(RoutingStrategy::LatencyBasedRouting, "latency-based-routing")]
#[case::cost(RoutingStrategy::CostBasedRouting, "cost-based-routing")]
#[case::usage_v2(RoutingStrategy::UsageBasedRoutingV2, "usage-based-routing-v2")]
#[case::usage(RoutingStrategy::UsageBasedRouting, "usage-based-routing")]
#[case::provider_budget(RoutingStrategy::ProviderBudgetRouting, "provider-budget-routing")]
fn wire_names_round_trip(#[case] strategy: RoutingStrategy, #[case] name: &str) {
    assert_eq!(strategy.wire_name(), name);
    assert_eq!(RoutingStrategy::parse(name).unwrap(), strategy);
}

#[test]
fn fixed_strategies_ignore_arguments() {
    let usage = Some(SelectorArgs::Usage(UsageArgs { ttl_seconds: 5 }));
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::SimpleShuffle, usage).unwrap(),
        SelectorArgs::Fixed
    );
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::Lar1, usage).unwrap(),
        SelectorArgs::Fixed
    );
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::LeastBusy, usage).unwrap(),
        SelectorArgs::Fixed
    );
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::CostBasedRouting, usage).unwrap(),
        SelectorArgs::Fixed
    );
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::ProviderBudgetRouting, usage).unwrap(),
        SelectorArgs::Fixed
    );
    assert_eq!(
        SelectorArgs::defaults_for(RoutingStrategy::SimpleShuffle),
        SelectorArgs::Fixed
    );
    assert_eq!(
        SelectorArgs::defaults_for(RoutingStrategy::LeastBusy),
        SelectorArgs::Fixed
    );
    assert_eq!(
        SelectorArgs::defaults_for(RoutingStrategy::CostBasedRouting),
        SelectorArgs::Fixed
    );
    assert_eq!(SelectorArgs::Fixed.kind_name(), "no routing arguments");
}

#[test]
fn usage_arguments_default_and_validate() {
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::UsageBasedRouting, None).unwrap(),
        SelectorArgs::Usage(UsageArgs { ttl_seconds: 60 })
    );
    assert_eq!(
        SelectorArgs::for_strategy(
            RoutingStrategy::UsageBasedRoutingV2,
            Some(SelectorArgs::Usage(UsageArgs { ttl_seconds: 5 }))
        )
        .unwrap(),
        SelectorArgs::Usage(UsageArgs { ttl_seconds: 5 })
    );
    assert!(matches!(
        SelectorArgs::for_strategy(
            RoutingStrategy::UsageBasedRoutingV2,
            Some(latency_arguments(None))
        ),
        Err(StrategyDispatchError::ArgsMismatch { .. })
    ));
    assert_eq!(
        SelectorArgs::defaults_for(RoutingStrategy::UsageBasedRoutingV2),
        SelectorArgs::Usage(UsageArgs { ttl_seconds: 60 })
    );
    assert_eq!(
        SelectorArgs::Usage(UsageArgs { ttl_seconds: 5 }).kind_name(),
        "usage-based arguments"
    );
}

#[test]
fn latency_arguments_default_and_validate() {
    let defaults = latency_arguments(None);
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::LatencyBasedRouting, None).unwrap(),
        defaults
    );
    assert_eq!(
        SelectorArgs::defaults_for(RoutingStrategy::LatencyBasedRouting),
        defaults
    );
    let custom = latency_arguments(Some(0.5_f64));
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::LatencyBasedRouting, Some(custom)).unwrap(),
        custom
    );
    assert_eq!(
        SelectorArgs::for_strategy(
            RoutingStrategy::LatencyBasedRouting,
            Some(latency_arguments(Some(1.0_f64)))
        )
        .unwrap(),
        latency_arguments(Some(1.0_f64))
    );
    assert_eq!(defaults.kind_name(), "latency-based arguments");
    assert!(matches!(
        SelectorArgs::for_strategy(
            RoutingStrategy::LatencyBasedRouting,
            Some(SelectorArgs::Usage(UsageArgs { ttl_seconds: 5 }))
        ),
        Err(StrategyDispatchError::ArgsMismatch { .. })
    ));
}

#[test]
fn argument_mismatches_name_the_fields() {
    let mismatch = SelectorArgs::for_strategy(
        RoutingStrategy::UsageBasedRoutingV2,
        Some(latency_arguments(Some(0.5_f64))),
    )
    .unwrap_err();
    let StrategyDispatchError::ArgsMismatch {
        expected, supplied, ..
    } = mismatch
    else {
        panic!("latency arguments do not fit usage-based routing");
    };
    assert_eq!(expected, "usage-based arguments");
    assert_eq!(supplied, "latency-based arguments");
}

#[test]
fn binding_edges_refuse_missing_inputs() {
    let dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    assert!(!bind_override_selector(
        &dispatch,
        RoutingStrategy::LeastBusy,
        None,
        &RoutingGroups::default(),
        None
    ));
    assert!(!bind_override_selector(
        &dispatch,
        RoutingStrategy::LeastBusy,
        Some(SelectorId(0)),
        &RoutingGroups::default(),
        None
    ));
}
