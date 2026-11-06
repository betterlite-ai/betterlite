#![cfg(test)]

use std::collections::BTreeSet;

use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use crate::simple_shuffle::Candidate;
use crate::strategy_dispatch::args::{LatencyArgs, SelectorArgs, UsageArgs};
use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::groups::{GroupStrategy, RoutingGroupRoute, RoutingGroups};
use crate::strategy_dispatch::override_resolution::{
    bind_override_selector, globally_registered_strategies, override_pre_call_target,
    override_selector, request_override, resolve,
};
use crate::strategy_dispatch::selection::{
    Selection, SelectorCall, read_prefetch_plan, select, selector_call,
};
use crate::strategy_dispatch::selector::{SelectorId, SelectorState};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{
    ArgsUpdate, OverrideDecision, RequestBindings, ResolutionSource, RoutingRequest,
    StrategyOverride,
};

const SEED: u64 = 7;

fn new_dispatch(strategy: RoutingStrategy) -> StrategyDispatch {
    StrategyDispatch::new(strategy, None).unwrap()
}

fn callback_strategies(state: &SelectorState) -> Vec<RoutingStrategy> {
    state
        .callbacks
        .iter()
        .map(|selector| selector.strategy)
        .collect()
}

fn input_callback_strategies(state: &SelectorState) -> Vec<RoutingStrategy> {
    state
        .input_callbacks
        .iter()
        .map(|selector| selector.strategy)
        .collect()
}

fn request_for(model: &'static str) -> RoutingRequest<'static> {
    RoutingRequest {
        resolved_model: model,
        strategy: StrategyOverride::Absent,
        bindings: None,
    }
}

fn cheap_groups() -> RoutingGroups {
    RoutingGroups::new(vec![
        RoutingGroupRoute {
            name: "cheap".to_owned(),
            models: vec!["fast".to_owned()],
            strategy: GroupStrategy::Routing(RoutingStrategy::LeastBusy),
            selector: None,
        },
        RoutingGroupRoute {
            name: "prio".to_owned(),
            models: vec!["fast".to_owned(), "slow".to_owned()],
            strategy: GroupStrategy::Priority,
            selector: None,
        },
    ])
    .unwrap()
}

enum Behavior {
    Init {
        strategy: RoutingStrategy,
        selector: Option<RoutingStrategy>,
        callbacks: Vec<RoutingStrategy>,
        input_callbacks: Vec<RoutingStrategy>,
    },
    CostIgnoresArguments,
    ApplyArgsRebuilds,
    ApplyArgsKeepsPrevious,
    ApplyArgsLar1,
    ApplyArgsNoSelector,
    RequestOverrideHonored,
    RequestOverrideMatchesDefault,
    RequestOverrideIgnored {
        raw: StrategyOverride<'static>,
        raw_text: Option<&'static str>,
    },
    Groups,
    PrefetchPlan,
    BindSkipsGlobal,
    GroupSelectors,
}

#[rstest]
#[case::init_simple_shuffle(Behavior::Init { strategy: RoutingStrategy::SimpleShuffle, selector: None, callbacks: vec![], input_callbacks: vec![] })]
#[case::init_least_busy(Behavior::Init { strategy: RoutingStrategy::LeastBusy, selector: Some(RoutingStrategy::LeastBusy), callbacks: vec![RoutingStrategy::LeastBusy], input_callbacks: vec![RoutingStrategy::LeastBusy] })]
#[case::init_usage_based(Behavior::Init { strategy: RoutingStrategy::UsageBasedRouting, selector: Some(RoutingStrategy::UsageBasedRouting), callbacks: vec![RoutingStrategy::UsageBasedRouting], input_callbacks: vec![] })]
#[case::init_usage_based_v2(Behavior::Init { strategy: RoutingStrategy::UsageBasedRoutingV2, selector: Some(RoutingStrategy::UsageBasedRoutingV2), callbacks: vec![RoutingStrategy::UsageBasedRoutingV2], input_callbacks: vec![] })]
#[case::init_latency(Behavior::Init { strategy: RoutingStrategy::LatencyBasedRouting, selector: Some(RoutingStrategy::LatencyBasedRouting), callbacks: vec![RoutingStrategy::LatencyBasedRouting], input_callbacks: vec![] })]
#[case::init_cost(Behavior::Init { strategy: RoutingStrategy::CostBasedRouting, selector: Some(RoutingStrategy::CostBasedRouting), callbacks: vec![RoutingStrategy::CostBasedRouting], input_callbacks: vec![] })]
#[case::init_lar1(Behavior::Init { strategy: RoutingStrategy::Lar1, selector: None, callbacks: vec![], input_callbacks: vec![] })]
#[case::init_provider_budget(Behavior::Init { strategy: RoutingStrategy::ProviderBudgetRouting, selector: None, callbacks: vec![], input_callbacks: vec![] })]
#[case::cost_based_ignores_arguments(Behavior::CostIgnoresArguments)]
#[case::updated_args_rebuild(Behavior::ApplyArgsRebuilds)]
#[case::updated_args_keep_previous(Behavior::ApplyArgsKeepsPrevious)]
#[case::updated_args_lar1(Behavior::ApplyArgsLar1)]
#[case::updated_args_without_selector(Behavior::ApplyArgsNoSelector)]
#[case::request_override_honored(Behavior::RequestOverrideHonored)]
#[case::request_override_matches_default(Behavior::RequestOverrideMatchesDefault)]
#[case::request_override_ignores_lar1(Behavior::RequestOverrideIgnored { raw: StrategyOverride::Text("lar1"), raw_text: Some("lar1") })]
#[case::request_override_ignores_unknown(Behavior::RequestOverrideIgnored { raw: StrategyOverride::Text("nope"), raw_text: Some("nope") })]
#[case::request_override_ignores_non_text(Behavior::RequestOverrideIgnored { raw: StrategyOverride::NotText, raw_text: None })]
#[case::group_resolution(Behavior::Groups)]
#[case::read_prefetch_plan(Behavior::PrefetchPlan)]
#[case::binding_skips_globally_registered(Behavior::BindSkipsGlobal)]
#[case::group_selectors_stay_registered(Behavior::GroupSelectors)]
fn strategy_dispatch_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::Init {
            strategy,
            selector,
            callbacks,
            input_callbacks,
        } => {
            let state = new_dispatch(strategy).state();
            assert_eq!(state.selector.map(|selected| selected.strategy), selector);
            assert_eq!(callback_strategies(&state), callbacks);
            assert_eq!(input_callback_strategies(&state), input_callbacks);
        }
        Behavior::CostIgnoresArguments => {
            let arguments = SelectorArgs::Usage(UsageArgs { ttl_seconds: 5 });
            let dispatch =
                StrategyDispatch::new(RoutingStrategy::CostBasedRouting, Some(arguments)).unwrap();
            assert_eq!(
                dispatch.state().selector.map(|selector| selector.args),
                Some(SelectorArgs::Fixed)
            );
        }
        Behavior::ApplyArgsRebuilds => {
            let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
            let previous = dispatch.state().selector.unwrap().id;
            let update = dispatch
                .apply_updated_args(Some(SelectorArgs::Usage(UsageArgs { ttl_seconds: 7 })));
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
            let state = dispatch.state();
            assert_eq!(state.selector, Some(selector));
            assert_eq!(
                callback_strategies(&state),
                vec![RoutingStrategy::UsageBasedRoutingV2]
            );
            assert!(dispatch.selectors[previous.0].retired);
        }
        Behavior::ApplyArgsKeepsPrevious => {
            let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
            let before = dispatch.state();
            let update =
                dispatch.apply_updated_args(Some(SelectorArgs::Latency(LatencyArgs::default())));
            let ArgsUpdate::KeptPrevious { error } = update else {
                panic!("mismatched arguments must keep the previous selector");
            };
            assert!(matches!(error, StrategyDispatchError::ArgsMismatch { .. }));
            assert_eq!(dispatch.state(), before);
            assert_eq!(
                dispatch.args,
                SelectorArgs::Usage(UsageArgs { ttl_seconds: 60 })
            );
        }
        Behavior::ApplyArgsLar1 => {
            let mut dispatch = new_dispatch(RoutingStrategy::Lar1);
            assert!(matches!(
                dispatch.apply_updated_args(None),
                ArgsUpdate::Lar1Delegated
            ));
        }
        Behavior::ApplyArgsNoSelector => {
            let mut dispatch = new_dispatch(RoutingStrategy::SimpleShuffle);
            assert!(matches!(
                dispatch.apply_updated_args(None),
                ArgsUpdate::NoSelector
            ));
        }
        Behavior::RequestOverrideHonored => {
            let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
            let mut bindings = RequestBindings::default();
            let mut request = RoutingRequest {
                resolved_model: "gpt",
                strategy: StrategyOverride::Text("least-busy"),
                bindings: Some(&mut bindings),
            };
            let resolution = resolve(&mut dispatch, &mut request, &RoutingGroups::default());
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
        Behavior::RequestOverrideMatchesDefault => {
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
        Behavior::RequestOverrideIgnored { raw, raw_text } => {
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
        Behavior::Groups => {
            let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
            let groups = cheap_groups();
            let callable = resolve(&mut dispatch, &mut request_for("cheap"), &groups);
            assert_eq!(callable.strategy, RoutingStrategy::LeastBusy);
            assert_eq!(
                callable.source,
                ResolutionSource::RoutingGroup("cheap".to_owned())
            );
            let member = resolve(&mut dispatch, &mut request_for("fast"), &groups);
            assert_eq!(member.strategy, RoutingStrategy::LeastBusy);
            let priority = resolve(&mut dispatch, &mut request_for("prio"), &groups);
            assert_eq!(priority.strategy, RoutingStrategy::SimpleShuffle);
            assert_eq!(priority.selector, None);
            let default = resolve(&mut dispatch, &mut request_for("slow"), &groups);
            assert_eq!(default.strategy, RoutingStrategy::UsageBasedRoutingV2);
            assert_eq!(default.source, ResolutionSource::Default);
        }
        Behavior::PrefetchPlan => {
            let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
            let deployments = ["one".to_owned(), "two".to_owned()];
            let mut random = StdRng::seed_from_u64(SEED);
            let plan = read_prefetch_plan(
                &mut dispatch,
                &mut request_for("gpt"),
                &RoutingGroups::default(),
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
                    &RoutingGroups::default(),
                    &Vec::<String>::new(),
                )
                .is_none()
            );
            let mut simple = new_dispatch(RoutingStrategy::SimpleShuffle);
            let candidates = [Candidate {
                weight: 0,
                rpm: 0,
                tpm: 0,
            }];
            let outcome = select(
                &mut simple,
                &mut request_for("gpt"),
                &RoutingGroups::default(),
                &candidates,
                &mut random,
            )
            .unwrap();
            assert_eq!(outcome.selection, Selection::SimpleShuffle { index: 0 });
            let simple_plan = read_prefetch_plan(
                &mut simple,
                &mut request_for("gpt"),
                &RoutingGroups::default(),
                &deployments,
            )
            .unwrap();
            assert_eq!(simple_plan.usage_selector, None);
        }
        Behavior::GroupSelectors => {
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
            let after_group = dispatch.state();
            let registered = callback_strategies(&after_group)
                .iter()
                .filter(|strategy| **strategy == RoutingStrategy::UsageBasedRoutingV2)
                .count();
            assert_eq!(registered, 2);
        }
        Behavior::BindSkipsGlobal => {
            let mut dispatch = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
            let groups = cheap_groups();
            let mut bindings = RequestBindings::default();
            let mut request = RoutingRequest {
                resolved_model: "gpt",
                strategy: StrategyOverride::Text("least-busy"),
                bindings: Some(&mut bindings),
            };
            let resolution = resolve(&mut dispatch, &mut request, &groups);
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
    }
}

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

fn selector_call_table() {
    let selector = Some(SelectorId(0));
    assert_eq!(
        selector_call(RoutingStrategy::LeastBusy, selector),
        Some(SelectorCall::LeastBusy)
    );
    assert_eq!(
        selector_call(RoutingStrategy::UsageBasedRouting, selector),
        Some(SelectorCall::UsageBasedV1)
    );
    assert_eq!(
        selector_call(RoutingStrategy::UsageBasedRoutingV2, selector),
        Some(SelectorCall::UsageBasedV2)
    );
    assert_eq!(
        selector_call(RoutingStrategy::CostBasedRouting, selector),
        Some(SelectorCall::CostBased)
    );
    assert_eq!(
        selector_call(RoutingStrategy::LatencyBasedRouting, selector),
        Some(SelectorCall::LatencyBased)
    );
    assert_eq!(
        selector_call(RoutingStrategy::SimpleShuffle, selector),
        None
    );
    assert_eq!(selector_call(RoutingStrategy::Lar1, selector), None);
    assert_eq!(
        selector_call(RoutingStrategy::ProviderBudgetRouting, selector),
        None
    );
    assert_eq!(selector_call(RoutingStrategy::LeastBusy, None), None);
}

fn single_candidate_selection() {
    let mut dispatch = new_dispatch(RoutingStrategy::SimpleShuffle);
    let mut random = StdRng::seed_from_u64(SEED);
    let candidates = [Candidate {
        weight: 0,
        rpm: 0,
        tpm: 0,
    }];
    let outcome = select(
        &mut dispatch,
        &mut request_for("gpt"),
        &RoutingGroups::default(),
        &candidates,
        &mut random,
    )
    .unwrap();
    assert_eq!(outcome.selection, Selection::SimpleShuffle { index: 0 });
    let error = select(
        &mut dispatch,
        &mut request_for("gpt"),
        &RoutingGroups::default(),
        &[],
        &mut random,
    )
    .unwrap_err();
    assert!(matches!(error, StrategyDispatchError::Shuffle { .. }));
}

fn override_selector_boundaries() {
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
fn strategy_dispatch_boundaries() {
    error_fields_are_typed();
    unregister_retires_by_identity();
    selector_call_table();
    single_candidate_selection();
    override_selector_boundaries();
    duplicate_groups_are_refused();
    init_clears_override_cache();
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
            Some(SelectorArgs::Latency(LatencyArgs::default()))
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
    let defaults = SelectorArgs::Latency(LatencyArgs {
        ttl_seconds: 3600.0,
        lowest_latency_buffer: 0.0,
        max_latency_list_size: 10,
        ttft_percentile: None,
    });
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::LatencyBasedRouting, None).unwrap(),
        defaults
    );
    let custom = SelectorArgs::Latency(LatencyArgs {
        ttl_seconds: 12.5,
        lowest_latency_buffer: 0.25,
        max_latency_list_size: 4,
        ttft_percentile: Some(0.5_f64),
    });
    assert_eq!(
        SelectorArgs::for_strategy(RoutingStrategy::LatencyBasedRouting, Some(custom)).unwrap(),
        custom
    );
    assert_eq!(
        SelectorArgs::defaults_for(RoutingStrategy::LatencyBasedRouting),
        defaults
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
    let custom = SelectorArgs::Latency(LatencyArgs {
        ttl_seconds: 12.5,
        lowest_latency_buffer: 0.25,
        max_latency_list_size: 4,
        ttft_percentile: Some(0.5_f64),
    });
    let mismatch =
        SelectorArgs::for_strategy(RoutingStrategy::UsageBasedRoutingV2, Some(custom)).unwrap_err();
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
fn least_busy_delegates_its_selector() {
    let candidates = [Candidate {
        weight: 0,
        rpm: 0,
        tpm: 0,
    }];
    let mut random = StdRng::seed_from_u64(SEED);
    let mut dispatch = new_dispatch(RoutingStrategy::LeastBusy);
    let default_selector = dispatch.state().selector.unwrap().id;
    let outcome = select(
        &mut dispatch,
        &mut request_for("gpt"),
        &RoutingGroups::default(),
        &candidates,
        &mut random,
    )
    .unwrap();
    assert_eq!(
        outcome.selection,
        Selection::Selector {
            selector: default_selector,
            call: SelectorCall::LeastBusy,
        }
    );
}

#[test]
fn usage_v2_delegates_and_lar1_has_no_selector() {
    let candidates = [Candidate {
        weight: 0,
        rpm: 0,
        tpm: 0,
    }];
    let mut random = StdRng::seed_from_u64(SEED);
    let mut usage = new_dispatch(RoutingStrategy::UsageBasedRoutingV2);
    let usage_selector = usage.state().selector.unwrap().id;
    let usage_outcome = select(
        &mut usage,
        &mut request_for("gpt"),
        &RoutingGroups::default(),
        &candidates,
        &mut random,
    )
    .unwrap();
    assert_eq!(
        usage_outcome.selection,
        Selection::Selector {
            selector: usage_selector,
            call: SelectorCall::UsageBasedV2,
        }
    );
    let mut lar1 = new_dispatch(RoutingStrategy::Lar1);
    let lar1_outcome = select(
        &mut lar1,
        &mut request_for("gpt"),
        &RoutingGroups::default(),
        &candidates,
        &mut random,
    )
    .unwrap();
    assert_eq!(lar1_outcome.selection, Selection::NoSelector);
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
