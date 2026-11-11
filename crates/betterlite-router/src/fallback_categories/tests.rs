#![cfg(test)]

use rstest::rstest;

use super::contracts::{
    ContentPolicyResponse, FallbackChain, FallbackFailure, FallbackSuccessor, FallbackTarget,
    MetadataBucket, OrderExternal, OrderFallbacks, OrdinaryFallbacks, RefusalRequest,
    RefusalResponse, StopReason, SuccessorRequest, TargetSource, WeightedFailover,
};
use super::decide::decide;
use super::gates::{
    has_content_policy_fallback, has_default_fallbacks, raises_anthropic_refusal,
    raises_content_policy_error, refusal_fallback_available,
};
use crate::weighted_failover::FailoverInputs;

fn target(model: &str) -> FallbackTarget {
    FallbackTarget {
        model: model.to_owned(),
        target_order: None,
        excluded_deployment_ids: Vec::new(),
    }
}

fn ordered(model: &str, order: i64) -> FallbackTarget {
    FallbackTarget {
        model: model.to_owned(),
        target_order: Some(order),
        excluded_deployment_ids: Vec::new(),
    }
}

fn resolved(targets: &[FallbackTarget], has_unattempted: bool) -> FallbackChain<'_> {
    FallbackChain::Resolved {
        targets,
        has_unattempted,
    }
}

fn no_order() -> OrderFallbacks<'static> {
    OrderFallbacks {
        model_group: "requested",
        deployment_orders: &[],
        current_target: None,
        external: OrderExternal::Absent,
    }
}

fn no_failover() -> WeightedFailover<'static> {
    WeightedFailover {
        enabled: false,
        inputs: FailoverInputs {
            strategy: "simple-shuffle",
            failed_deployment_id: None,
            model_group: "requested",
            deployment_ids: &[],
            cooldown_ids: &[],
        },
        metadata_key: MetadataBucket::Metadata,
        prior_excluded_ids: &[],
    }
}

fn request_with(failure: FallbackFailure, ordinary: OrdinaryFallbacks<'_>) -> SuccessorRequest<'_> {
    SuccessorRequest {
        failure,
        original_model_group: Some("requested"),
        disable_fallbacks: false,
        guardrail_intervention: false,
        order: no_order(),
        weighted_failover: no_failover(),
        context_window: FallbackChain::Absent,
        content_policy: FallbackChain::Absent,
        ordinary,
    }
}

fn refusal_request<'plan>(
    content_policy: FallbackChain<'plan>,
    ordinary: &'plan OrdinaryFallbacks<'plan>,
    router_fallbacks: &'plan [&'plan str],
) -> RefusalRequest<'plan> {
    RefusalRequest {
        fallbacks_disabled: false,
        content_policy,
        ordinary,
        router_fallbacks,
    }
}

fn failover<'plan>(
    deployments: &'plan [String],
    prior: &'plan [String],
) -> WeightedFailover<'plan> {
    WeightedFailover {
        enabled: true,
        inputs: FailoverInputs {
            strategy: "simple-shuffle",
            failed_deployment_id: Some("a"),
            model_group: "requested",
            deployment_ids: deployments,
            cooldown_ids: &[],
        },
        metadata_key: MetadataBucket::Metadata,
        prior_excluded_ids: prior,
    }
}

fn context_chain_wins_over_content_chain() {
    let window_targets = [target("window-model")];
    let policy_targets = [target("policy-model")];
    let ordinary_targets = [target("ordinary-model")];
    let mut request = request_with(
        FallbackFailure::ContextWindow,
        OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
    );
    request.context_window = resolved(&window_targets, true);
    request.content_policy = resolved(&policy_targets, true);
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunTargets {
            source: TargetSource::ContextWindow,
            targets: window_targets.to_vec(),
        }
    );
}

fn content_chain_wins_for_a_content_failure() {
    let policy_targets = [target("policy-model")];
    let ordinary_targets = [target("ordinary-model")];
    let mut request = request_with(
        FallbackFailure::ContentPolicy,
        OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
    );
    request.content_policy = resolved(&policy_targets, true);
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunTargets {
            source: TargetSource::ContentPolicy,
            targets: policy_targets.to_vec(),
        }
    );
}

fn absent_category_chain_falls_through_to_the_ordinary_chain() {
    let ordinary_targets = [target("ordinary-model")];
    for failure in [
        FallbackFailure::ContextWindow,
        FallbackFailure::ContentPolicy,
    ] {
        let request = request_with(
            failure,
            OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
        );
        assert_eq!(
            decide(&request),
            FallbackSuccessor::RunTargets {
                source: TargetSource::Ordinary,
                targets: ordinary_targets.to_vec(),
            }
        );
    }
}

fn ordinary_failure_uses_the_ordinary_chain() {
    let ordinary_targets = [target("ordinary-model")];
    let request = request_with(
        FallbackFailure::Ordinary,
        OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
    );
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunTargets {
            source: TargetSource::Ordinary,
            targets: ordinary_targets.to_vec(),
        }
    );
}

fn order_levels_run_before_the_ordinary_chain() {
    let ordinary_targets = [target("ordinary-model")];
    let mut request = request_with(
        FallbackFailure::Ordinary,
        OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
    );
    request.order = OrderFallbacks {
        model_group: "requested",
        deployment_orders: &[Some(1), Some(2)],
        current_target: None,
        external: OrderExternal::Targets(&ordinary_targets),
    };
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunOrder {
            targets: vec![ordered("requested", 2), target("ordinary-model")],
            client_fallbacks_after: false,
        }
    );
}

fn order_fallbacks_are_skipped_for_category_failures() {
    let policy_targets = [target("policy-model")];
    let mut request = request_with(FallbackFailure::ContentPolicy, OrdinaryFallbacks::Absent);
    request.context_window = resolved(&policy_targets, true);
    request.content_policy = resolved(&policy_targets, true);
    request.order = OrderFallbacks {
        model_group: "requested",
        deployment_orders: &[Some(1), Some(2)],
        current_target: None,
        external: OrderExternal::Targets(&policy_targets),
    };
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunTargets {
            source: TargetSource::ContentPolicy,
            targets: policy_targets.to_vec(),
        }
    );
}

fn resumed_order_level_starts_after_the_current_target() {
    let mut request = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    request.order = OrderFallbacks {
        model_group: "requested",
        deployment_orders: &[Some(3), Some(1), Some(2)],
        current_target: Some(2),
        external: OrderExternal::Absent,
    };
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunOrder {
            targets: vec![ordered("requested", 3)],
            client_fallbacks_after: false,
        }
    );
}

fn client_fallbacks_run_as_given() {
    let request = request_with(
        FallbackFailure::Ordinary,
        OrdinaryFallbacks::NonStandard(FallbackChain::Unresolved),
    );
    assert_eq!(decide(&request), FallbackSuccessor::RunClientFallbacks);
}

fn client_fallbacks_follow_order_entries() {
    let mut request = request_with(
        FallbackFailure::Ordinary,
        OrdinaryFallbacks::NonStandard(FallbackChain::Unresolved),
    );
    request.order = OrderFallbacks {
        model_group: "requested",
        deployment_orders: &[Some(1), Some(2)],
        current_target: None,
        external: OrderExternal::ClientFallbacks,
    };
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunOrder {
            targets: vec![ordered("requested", 2)],
            client_fallbacks_after: true,
        }
    );
}

fn weighted_failover_plans_the_same_group_retry() {
    let deployments = [String::from("a"), String::from("b")];
    let prior = [String::from("c")];
    let mut request = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    request.weighted_failover = failover(&deployments, &prior);
    let written = vec![String::from("c"), String::from("a")];
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunWeightedFailover {
            targets: vec![FallbackTarget {
                model: "requested".to_owned(),
                target_order: None,
                excluded_deployment_ids: written.clone(),
            }],
            excluded_deployment_ids: written,
        }
    );
}

fn weighted_failover_runs_after_order_before_generic_fallbacks() {
    let deployments = [String::from("a"), String::from("b")];
    let mut request = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    request.order = OrderFallbacks {
        model_group: "requested",
        deployment_orders: &[Some(1), Some(2)],
        current_target: None,
        external: OrderExternal::Absent,
    };
    request.weighted_failover = failover(&deployments, &[]);
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunOrder {
            targets: vec![ordered("requested", 2)],
            client_fallbacks_after: false,
        }
    );
}

fn weighted_failover_is_skipped_for_category_failures() {
    let deployments = [String::from("a"), String::from("b")];
    let mut request = request_with(FallbackFailure::ContextWindow, OrdinaryFallbacks::Absent);
    request.context_window = FallbackChain::Unresolved;
    request.weighted_failover = failover(&deployments, &[]);
    assert_eq!(
        decide(&request),
        FallbackSuccessor::Stop(StopReason::NoContextWindowChain)
    );
}

fn early_stops_raise_the_original_exception() {
    let mut request = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    request.disable_fallbacks = true;
    assert_eq!(
        decide(&request),
        FallbackSuccessor::Stop(StopReason::FallbacksDisabled)
    );
    let mut without_group = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    without_group.original_model_group = None;
    assert_eq!(
        decide(&without_group),
        FallbackSuccessor::Stop(StopReason::MissingModelGroup)
    );
    let mut guarded = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    guarded.guardrail_intervention = true;
    assert_eq!(
        decide(&guarded),
        FallbackSuccessor::Stop(StopReason::GuardrailIntervention)
    );
}

fn content_policy_raises_only_for_a_filtered_or_empty_response() {
    let ordinary = OrdinaryFallbacks::Absent;
    let request = refusal_request(FallbackChain::Absent, &ordinary, &["*"]);
    let choices = |finish_reason| ContentPolicyResponse {
        has_choices: true,
        first_finish_reason: finish_reason,
    };
    assert!(raises_content_policy_error(
        &choices(Some("content_filter")),
        &request.content_policy,
        request.router_fallbacks
    ));
    assert!(!raises_content_policy_error(
        &choices(Some("length")),
        &request.content_policy,
        request.router_fallbacks
    ));
    assert!(!raises_content_policy_error(
        &choices(None),
        &request.content_policy,
        request.router_fallbacks
    ));
    assert!(raises_content_policy_error(
        &ContentPolicyResponse {
            has_choices: false,
            first_finish_reason: None,
        },
        &request.content_policy,
        request.router_fallbacks
    ));
}

fn anthropic_refusal_needs_the_messages_function_and_stop_details() {
    let ordinary = OrdinaryFallbacks::Absent;
    let request = refusal_request(FallbackChain::Absent, &ordinary, &["*"]);
    assert!(raises_anthropic_refusal(
        "anthropic_messages",
        RefusalResponse::RefusalWithStopDetails,
        &request
    ));
    assert!(!raises_anthropic_refusal(
        "completion",
        RefusalResponse::RefusalWithStopDetails,
        &request
    ));
    assert!(!raises_anthropic_refusal(
        "anthropic_messages",
        RefusalResponse::RefusalWithoutStopDetails,
        &request
    ));
    assert!(!raises_anthropic_refusal(
        "anthropic_messages",
        RefusalResponse::NotRefusal,
        &request
    ));
}

#[rstest]
#[case::context_chain_wins_over_content_chain(context_chain_wins_over_content_chain)]
#[case::content_chain_wins_for_a_content_failure(content_chain_wins_for_a_content_failure)]
#[case::absent_category_chain_falls_through_to_the_ordinary_chain(
    absent_category_chain_falls_through_to_the_ordinary_chain
)]
#[case::ordinary_failure_uses_the_ordinary_chain(ordinary_failure_uses_the_ordinary_chain)]
#[case::order_levels_run_before_the_ordinary_chain(order_levels_run_before_the_ordinary_chain)]
#[case::order_fallbacks_are_skipped_for_category_failures(
    order_fallbacks_are_skipped_for_category_failures
)]
#[case::resumed_order_level_starts_after_the_current_target(
    resumed_order_level_starts_after_the_current_target
)]
#[case::client_fallbacks_run_as_given(client_fallbacks_run_as_given)]
#[case::client_fallbacks_follow_order_entries(client_fallbacks_follow_order_entries)]
#[case::weighted_failover_plans_the_same_group_retry(weighted_failover_plans_the_same_group_retry)]
#[case::weighted_failover_runs_after_order_before_generic_fallbacks(
    weighted_failover_runs_after_order_before_generic_fallbacks
)]
#[case::weighted_failover_is_skipped_for_category_failures(
    weighted_failover_is_skipped_for_category_failures
)]
#[case::early_stops_raise_the_original_exception(early_stops_raise_the_original_exception)]
#[case::content_policy_raises_only_for_a_filtered_or_empty_response(
    content_policy_raises_only_for_a_filtered_or_empty_response
)]
#[case::anthropic_refusal_needs_the_messages_function_and_stop_details(
    anthropic_refusal_needs_the_messages_function_and_stop_details
)]
fn fallback_categories_behaviors(#[case] scenario: fn()) {
    scenario();
}

fn order_phase_needs_two_distinct_levels() {
    let ordinary_targets = [target("ordinary-model")];
    let none: &[Option<i64>] = &[];
    let one: &[Option<i64>] = &[Some(1)];
    for orders in [none, one] {
        let mut request = request_with(
            FallbackFailure::Ordinary,
            OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
        );
        request.order = OrderFallbacks {
            model_group: "requested",
            deployment_orders: orders,
            current_target: None,
            external: OrderExternal::Absent,
        };
        assert_eq!(
            decide(&request),
            FallbackSuccessor::RunTargets {
                source: TargetSource::Ordinary,
                targets: ordinary_targets.to_vec(),
            }
        );
    }
}

fn order_phase_with_no_remaining_level_or_external_falls_through() {
    let levels = [Some(1), Some(2)];
    let mut request = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    request.order = OrderFallbacks {
        model_group: "requested",
        deployment_orders: &levels,
        current_target: Some(2),
        external: OrderExternal::Absent,
    };
    assert_eq!(
        decide(&request),
        FallbackSuccessor::Stop(StopReason::NoFallbackChain)
    );
}

fn resolved_empty_chain_still_reaches_the_executor() {
    let empty: [FallbackTarget; 0] = [];
    let mut request = request_with(FallbackFailure::ContextWindow, OrdinaryFallbacks::Absent);
    request.context_window = resolved(&empty, false);
    assert_eq!(
        decide(&request),
        FallbackSuccessor::RunTargets {
            source: TargetSource::ContextWindow,
            targets: Vec::new(),
        }
    );
}

fn unresolved_category_chain_stops_even_with_an_ordinary_chain() {
    let ordinary_targets = [target("ordinary-model")];
    let mut window = request_with(
        FallbackFailure::ContextWindow,
        OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
    );
    window.context_window = FallbackChain::Unresolved;
    assert_eq!(
        decide(&window),
        FallbackSuccessor::Stop(StopReason::NoContextWindowChain)
    );
    let mut policy = request_with(
        FallbackFailure::ContentPolicy,
        OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true)),
    );
    policy.content_policy = FallbackChain::Unresolved;
    assert_eq!(
        decide(&policy),
        FallbackSuccessor::Stop(StopReason::NoContentPolicyChain)
    );
}

fn default_fallbacks_need_the_star_key() {
    assert!(!has_default_fallbacks(&[]));
    assert!(!has_default_fallbacks(&["gpt-4o"]));
    assert!(has_default_fallbacks(&["gpt-4o", "*"]));
}

fn empty_resolved_content_chain_blocks_the_raise() {
    let empty: [FallbackTarget; 0] = [];
    assert!(!has_content_policy_fallback(
        &resolved(&empty, false),
        &["*"]
    ));
}

fn attempted_only_content_chain_blocks_the_raise() {
    let policy_targets = [target("policy-model")];
    assert!(!has_content_policy_fallback(
        &resolved(&policy_targets, false),
        &["*"]
    ));
    assert!(has_content_policy_fallback(
        &resolved(&policy_targets, true),
        &["*"]
    ));
}

fn absent_content_policy_chain_uses_the_default_fallback() {
    assert!(has_content_policy_fallback(&FallbackChain::Absent, &["*"]));
    assert!(!has_content_policy_fallback(
        &FallbackChain::Absent,
        &["group"]
    ));
    assert!(!has_content_policy_fallback(
        &FallbackChain::Unresolved,
        &["*"]
    ));
}

fn refusal_gate_respects_disabled_and_unresolved_chains() {
    let ordinary = OrdinaryFallbacks::Absent;
    let mut disabled = refusal_request(FallbackChain::Absent, &ordinary, &["*"]);
    disabled.fallbacks_disabled = true;
    assert!(!refusal_fallback_available(&disabled));
    let unresolved = refusal_request(FallbackChain::Unresolved, &ordinary, &["*"]);
    assert!(!refusal_fallback_available(&unresolved));
}

fn refusal_gate_falls_back_to_the_ordinary_chain() {
    let ordinary_targets = [target("ordinary-model")];
    let attempted_ordinary = OrdinaryFallbacks::Standard(resolved(&ordinary_targets, false));
    let pending_ordinary = OrdinaryFallbacks::Standard(resolved(&ordinary_targets, true));
    let client_targets = [target("client-model")];
    let client_ordinary = OrdinaryFallbacks::NonStandard(resolved(&client_targets, true));
    let cases = [
        (
            refusal_request(FallbackChain::Absent, &attempted_ordinary, &[]),
            false,
        ),
        (
            refusal_request(FallbackChain::Absent, &pending_ordinary, &[]),
            true,
        ),
        (
            refusal_request(FallbackChain::Absent, &client_ordinary, &[]),
            true,
        ),
    ];
    for (request, expected) in cases {
        assert_eq!(refusal_fallback_available(&request), expected);
    }
}

fn refusal_gate_uses_the_content_policy_chain_first() {
    let empty: [FallbackTarget; 0] = [];
    let ordinary = OrdinaryFallbacks::Absent;
    let spent_policy = refusal_request(resolved(&empty, false), &ordinary, &["*"]);
    assert!(!refusal_fallback_available(&spent_policy));
    let policy_targets = [target("policy-model")];
    let pending_policy = refusal_request(resolved(&policy_targets, true), &ordinary, &[]);
    assert!(refusal_fallback_available(&pending_policy));
}

fn weighted_failover_needs_a_remaining_deployment_and_simple_shuffle() {
    let deployments = [String::from("a")];
    let mut request = request_with(FallbackFailure::Ordinary, OrdinaryFallbacks::Absent);
    request.weighted_failover = failover(&deployments, &[]);
    assert_eq!(
        decide(&request),
        FallbackSuccessor::Stop(StopReason::NoFallbackChain)
    );
    let other = [String::from("a"), String::from("b")];
    request.weighted_failover.inputs.deployment_ids = &other;
    request.weighted_failover.inputs.strategy = "least-busy";
    assert_eq!(
        decide(&request),
        FallbackSuccessor::Stop(StopReason::NoFallbackChain)
    );
    request.weighted_failover.enabled = false;
    request.weighted_failover.inputs.strategy = "simple-shuffle";
    assert_eq!(
        decide(&request),
        FallbackSuccessor::Stop(StopReason::NoFallbackChain)
    );
}

#[rstest]
#[case::order_phase_needs_two_distinct_levels(order_phase_needs_two_distinct_levels)]
#[case::order_phase_with_no_remaining_level_or_external_falls_through(
    order_phase_with_no_remaining_level_or_external_falls_through
)]
#[case::resolved_empty_chain_still_reaches_the_executor(
    resolved_empty_chain_still_reaches_the_executor
)]
#[case::unresolved_category_chain_stops_even_with_an_ordinary_chain(
    unresolved_category_chain_stops_even_with_an_ordinary_chain
)]
#[case::default_fallbacks_need_the_star_key(default_fallbacks_need_the_star_key)]
#[case::empty_resolved_content_chain_blocks_the_raise(
    empty_resolved_content_chain_blocks_the_raise
)]
#[case::attempted_only_content_chain_blocks_the_raise(
    attempted_only_content_chain_blocks_the_raise
)]
#[case::absent_content_policy_chain_uses_the_default_fallback(
    absent_content_policy_chain_uses_the_default_fallback
)]
#[case::refusal_gate_respects_disabled_and_unresolved_chains(
    refusal_gate_respects_disabled_and_unresolved_chains
)]
#[case::refusal_gate_falls_back_to_the_ordinary_chain(
    refusal_gate_falls_back_to_the_ordinary_chain
)]
#[case::refusal_gate_uses_the_content_policy_chain_first(
    refusal_gate_uses_the_content_policy_chain_first
)]
#[case::weighted_failover_needs_a_remaining_deployment_and_simple_shuffle(
    weighted_failover_needs_a_remaining_deployment_and_simple_shuffle
)]
fn fallback_categories_boundaries(#[case] scenario: fn()) {
    scenario();
}
