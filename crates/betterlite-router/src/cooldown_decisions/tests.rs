#![cfg(test)]

use rstest::rstest;

use super::decision::{
    AllowedFailsCounter, CooldownDecision, CooldownRequest, DeploymentFacts, MinuteSamples, decide,
};
use super::failure::{ErrorStatus, FailureClass, FailureFacts, Seconds, TimeoutFacts};
use super::policy::{AllowedFailsPolicy, CooldownPolicy, FailureThresholds};

enum PolicyEntry {
    ContentPolicyViolation,
    BadRequest,
    RateLimit,
    ServiceUnavailable,
    BadGateway,
}

fn policy_with(entry: &PolicyEntry, value: u32) -> AllowedFailsPolicy {
    let mut policy = AllowedFailsPolicy::default();
    match entry {
        PolicyEntry::ContentPolicyViolation => {
            policy.content_policy_violation_error = Some(value);
        }
        PolicyEntry::BadRequest => {
            policy.bad_request_error = Some(value);
        }
        PolicyEntry::RateLimit => {
            policy.rate_limit_error = Some(value);
        }
        PolicyEntry::ServiceUnavailable => {
            policy.service_unavailable_error = Some(value);
        }
        PolicyEntry::BadGateway => {
            policy.bad_gateway_error = Some(value);
        }
    }
    policy
}

fn policy() -> CooldownPolicy {
    CooldownPolicy {
        allowed_fails: 3,
        default_allowed_fails: 3,
        cooldown_time: Seconds(5.0),
        disable_cooldowns: false,
        allowed_fails_policy: None,
        thresholds: FailureThresholds::default(),
    }
}

fn failure(class: FailureClass, status: ErrorStatus) -> FailureFacts {
    FailureFacts {
        class,
        status,
        advisor_orchestration: false,
        background_response_cost_poll: false,
        caller_timeout: None,
    }
}

fn deployment() -> DeploymentFacts {
    DeploymentFacts {
        id: "dep-1".to_owned(),
        model_group_size: 2,
        has_alternatives: false,
        provider_default: false,
        allowed_fails_policy: None,
        allowed_fails: None,
        cooldown_time: None,
    }
}

fn minute(successes: u64, failures: u64) -> MinuteSamples {
    MinuteSamples {
        successes,
        failures,
    }
}

fn run(
    policy: &CooldownPolicy,
    deployment: Option<&DeploymentFacts>,
    failure: &FailureFacts,
    minute: MinuteSamples,
    allowed_fails_before: u64,
) -> CooldownDecision {
    decide(&CooldownRequest {
        policy,
        deployment,
        failure,
        minute,
        allowed_fails_before,
        time_to_cooldown: None,
    })
}

fn decide_with_time(
    policy: &CooldownPolicy,
    deployment: &DeploymentFacts,
    failure: &FailureFacts,
    time_to_cooldown: Option<Seconds>,
) -> CooldownDecision {
    decide(&CooldownRequest {
        policy,
        deployment: Some(deployment),
        failure,
        minute: minute(0, 0),
        allowed_fails_before: 0,
        time_to_cooldown,
    })
}

fn counter_key(decision: &CooldownDecision) -> Option<&str> {
    decision
        .counter
        .as_ref()
        .map(|counter| counter.key.as_str())
}

fn counter_fails(decision: &CooldownDecision) -> Option<u64> {
    decision.counter.as_ref().map(|counter| counter.fails)
}

fn counter_ttl(decision: &CooldownDecision) -> Option<Seconds> {
    decision.counter.as_ref().map(|counter| counter.ttl_seconds)
}

fn case_deployment_policy_wins() -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails_policy = Some(policy_with(&PolicyEntry::BadRequest, 100));
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(&PolicyEntry::ContentPolicyViolation, 1));
    let failure = failure(FailureClass::ContentPolicyViolation, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 1)
}

fn case_deployment_policy_miss_falls_back_to_router() -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails_policy = Some(policy_with(&PolicyEntry::RateLimit, 2));
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(&PolicyEntry::BadGateway, 9));
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 2)
}

fn case_router_policy_applies() -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails = 10;
    router.allowed_fails_policy = Some(policy_with(&PolicyEntry::RateLimit, 2));
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(500));
    run(&router, Some(&deployment), &failure, minute(0, 0), 2)
}

fn case_generic_deployment_allowed_fails() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails = Some(1);
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 1)
}

fn case_generic_deployment_allowed_fails_single_group() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.model_group_size = 1;
    deployment.allowed_fails = Some(1);
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 1)
}

fn case_single_deployment_429() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.model_group_size = 1;
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(10, 1), 0)
}

fn case_multi_deployment_429() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_all_requests_failed_single_group() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.model_group_size = 1;
    let failure = failure(FailureClass::InternalServer, ErrorStatus::Code(500));
    run(&router, Some(&deployment), &failure, minute(0, 1000), 0)
}

fn case_ratio_above_threshold() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::InternalServer, ErrorStatus::Code(500));
    run(&router, Some(&deployment), &failure, minute(2, 4), 0)
}

fn case_ratio_at_threshold() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::InternalServer, ErrorStatus::Code(500));
    run(&router, Some(&deployment), &failure, minute(3, 3), 0)
}

fn case_non_retryable_status() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::Authentication, ErrorStatus::Code(401));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_low_traffic_ratio() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::InternalServer, ErrorStatus::Code(500));
    run(&router, Some(&deployment), &failure, minute(0, 4), 0)
}

fn case_advisor_orchestration() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let mut failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    failure.advisor_orchestration = true;
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_background_cost_poll() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let mut failure = failure(FailureClass::NotFound, ErrorStatus::Code(404));
    failure.background_response_cost_poll = true;
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_timeout(client_side_timeout: bool) -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let mut failure = failure(FailureClass::Timeout, ErrorStatus::Code(408));
    failure.caller_timeout = Some(TimeoutFacts {
        client_side_timeout,
        timeout_seconds: Some(Seconds(5.0)),
        elapsed_seconds: Some(Seconds(5.2)),
    });
    run(&router, Some(&deployment), &failure, minute(0, 5), 0)
}

fn case_disable_cooldowns() -> CooldownDecision {
    let mut router = policy();
    router.disable_cooldowns = true;
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_zero_time_to_cooldown() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    decide_with_time(&router, &deployment, &failure, Some(Seconds(0.0)))
}

fn case_provider_default_deployment() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.provider_default = true;
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_api_connection() -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::ApiConnection, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_deployment_cooldown_time() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(&PolicyEntry::ContentPolicyViolation, 1));
    deployment.cooldown_time = Some(Seconds(2.5));
    let failure = failure(FailureClass::ContentPolicyViolation, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 1)
}

fn case_zero_cooldown_time() -> CooldownDecision {
    let mut router = policy();
    router.cooldown_time = Seconds(0.0);
    router.allowed_fails = 0;
    router.default_allowed_fails = 0;
    router.allowed_fails_policy = Some(policy_with(&PolicyEntry::RateLimit, 0));
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(500));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_router_policy_does_not_override_400() -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails_policy = Some(policy_with(&PolicyEntry::BadRequest, 0));
    let deployment = deployment();
    let failure = failure(FailureClass::BadRequest, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_content_policy_falls_through_to_bad_request() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(&PolicyEntry::BadRequest, 0));
    let failure = failure(FailureClass::ContentPolicyViolation, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_mid_stream_matches_service_unavailable() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(&PolicyEntry::ServiceUnavailable, 0));
    let failure = failure(FailureClass::MidStreamFallback, ErrorStatus::Code(503));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_context_window_matches_bad_request() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(&PolicyEntry::BadRequest, 0));
    let failure = failure(FailureClass::ContextWindowExceeded, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_unknown_deployment() -> CooldownDecision {
    let router = policy();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, None, &failure, minute(0, 0), 0)
}

#[rstest]
#[case::deployment_policy_wins(
    case_deployment_policy_wins(),
    true,
    Some("deployment:dep-1:allowed_fails:ContentPolicyViolationError"),
    Some(2)
)]
#[case::deployment_policy_miss_falls_back_to_router(
    case_deployment_policy_miss_falls_back_to_router(),
    true,
    Some("deployment:dep-1:allowed_fails"),
    Some(3)
)]
#[case::router_policy_applies(
    case_router_policy_applies(),
    true,
    Some("deployment:dep-1:allowed_fails"),
    Some(3)
)]
#[case::generic_deployment_allowed_fails(
    case_generic_deployment_allowed_fails(),
    true,
    Some("deployment:dep-1:allowed_fails:generic"),
    Some(2)
)]
#[case::generic_deployment_allowed_fails_single_group(
    case_generic_deployment_allowed_fails_single_group(),
    false,
    None,
    None
)]
#[case::single_deployment_429(case_single_deployment_429(), false, None, None)]
#[case::multi_deployment_429(case_multi_deployment_429(), true, None, None)]
#[case::all_requests_failed_single_group(case_all_requests_failed_single_group(), true, None, None)]
#[case::ratio_above_threshold(case_ratio_above_threshold(), true, None, None)]
#[case::ratio_at_threshold(case_ratio_at_threshold(), false, None, None)]
#[case::non_retryable_status(case_non_retryable_status(), true, None, None)]
#[case::low_traffic_ratio(case_low_traffic_ratio(), false, None, None)]
#[case::advisor_orchestration(case_advisor_orchestration(), false, None, None)]
#[case::background_cost_poll(case_background_cost_poll(), false, None, None)]
#[case::caller_timeout(case_timeout(true), false, None, None)]
#[case::provider_timeout(case_timeout(false), true, None, None)]
#[case::disable_cooldowns(case_disable_cooldowns(), false, None, None)]
#[case::zero_time_to_cooldown(case_zero_time_to_cooldown(), false, None, None)]
#[case::provider_default_deployment(case_provider_default_deployment(), false, None, None)]
#[case::api_connection(case_api_connection(), false, None, None)]
#[case::deployment_cooldown_time(
    case_deployment_cooldown_time(),
    true,
    Some("deployment:dep-1:allowed_fails:ContentPolicyViolationError"),
    Some(2)
)]
#[case::zero_cooldown_time(
    case_zero_cooldown_time(),
    true,
    Some("deployment:dep-1:allowed_fails"),
    Some(1)
)]
#[case::router_policy_does_not_override_400(
    case_router_policy_does_not_override_400(),
    false,
    None,
    None
)]
#[case::content_policy_falls_through_to_bad_request(
    case_content_policy_falls_through_to_bad_request(),
    true,
    Some("deployment:dep-1:allowed_fails:ContentPolicyViolationError"),
    Some(1)
)]
#[case::mid_stream_matches_service_unavailable(
    case_mid_stream_matches_service_unavailable(),
    true,
    Some("deployment:dep-1:allowed_fails:MidStreamFallbackError"),
    Some(1)
)]
#[case::context_window_matches_bad_request(
    case_context_window_matches_bad_request(),
    true,
    Some("deployment:dep-1:allowed_fails:ContextWindowExceededError"),
    Some(1)
)]
#[case::unknown_deployment(case_unknown_deployment(), false, None, None)]
fn cooldown_decisions_behaviors(
    #[case] decision: CooldownDecision,
    #[case] cooldown: bool,
    #[case] key: Option<&str>,
    #[case] fails: Option<u64>,
) {
    assert_eq!(decision.cooldown, cooldown);
    assert_eq!(counter_key(&decision), key);
    assert_eq!(counter_fails(&decision), fails);
}

#[rstest]
#[case::deployment_cooldown_time_overrides_router(case_deployment_cooldown_time(), Seconds(2.5))]
#[case::default_router_cooldown_time(case_router_policy_applies(), Seconds(5.0))]
#[case::zero_cooldown_time_kept(case_zero_cooldown_time(), Seconds(0.0))]
fn cooldown_decisions_counter_ttl(#[case] decision: CooldownDecision, #[case] expected: Seconds) {
    assert_eq!(counter_ttl(&decision), Some(expected));
}

fn status_decision(status: ErrorStatus) -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::BadRequest, status);
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn counter_decision(before: u64) -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails = 3;
    router.default_allowed_fails = 0;
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), before)
}

#[rstest]
#[case::below_400_cooldowns(399, true)]
#[case::bad_request_stays_up(400, false)]
#[case::unauthorized_cooldowns(401, true)]
#[case::forbidden_stays_up(403, false)]
#[case::not_found_cooldowns(404, true)]
#[case::request_timeout_stays_up(408, false)]
#[case::conflict_stays_up(409, false)]
#[case::unprocessable_stays_up(422, false)]
#[case::rate_limit_cooldowns(429, true)]
#[case::last_client_error_stays_up(499, false)]
#[case::server_error_stays_up(500, false)]
#[case::top_server_error_stays_up(599, false)]
fn cooldown_decisions_status_boundaries(#[case] code: i64, #[case] cooldown: bool) {
    let decision = status_decision(ErrorStatus::Code(code));
    assert_eq!(decision.cooldown, cooldown);
    assert_eq!(decision.counter, None);
}

#[rstest]
#[case::empty_text_is_not_required(ErrorStatus::Text(String::new()), false)]
#[case::unparsable_text_is_required_then_not_retried(
    ErrorStatus::Text("abc".to_owned()),
    false
)]
#[case::text_unauthorized(ErrorStatus::Text("401".to_owned()), true)]
#[case::text_rate_limit(ErrorStatus::Text("429".to_owned()), true)]
#[case::text_server_error(ErrorStatus::Text(" 500 ".to_owned()), false)]
fn cooldown_decisions_text_status_boundaries(#[case] status: ErrorStatus, #[case] cooldown: bool) {
    let decision = status_decision(status);
    assert_eq!(decision.cooldown, cooldown);
}

#[rstest]
#[case::below_threshold(2, false, 3)]
#[case::at_threshold(3, true, 4)]
#[case::saturated_counter(u64::MAX, true, u64::MAX)]
fn cooldown_decisions_strict_counter_boundaries(
    #[case] before: u64,
    #[case] cooldown: bool,
    #[case] fails: u64,
) {
    let decision = counter_decision(before);
    assert_eq!(decision.cooldown, cooldown);
    assert_eq!(counter_fails(&decision), Some(fails));
}

#[rstest]
#[case::below_minimum_requests(0, 4, 2, false)]
#[case::at_minimum_requests(0, 5, 2, true)]
#[case::ratio_just_above_threshold(2, 3, 2, true)]
#[case::ratio_exactly_half(3, 3, 2, false)]
#[case::single_group_traffic_below(0, 999, 1, false)]
#[case::single_group_traffic_at_boundary(0, 1000, 1, true)]
#[case::saturated_total(u64::MAX, u64::MAX, 2, true)]
fn cooldown_decisions_ratio_boundaries(
    #[case] successes: u64,
    #[case] failures: u64,
    #[case] model_group_size: u32,
    #[case] cooldown: bool,
) {
    let router = policy();
    let mut deployment = deployment();
    deployment.model_group_size = model_group_size;
    let failure = failure(FailureClass::InternalServer, ErrorStatus::Code(500));
    let decision = run(
        &router,
        Some(&deployment),
        &failure,
        minute(successes, failures),
        0,
    );
    assert_eq!(decision.cooldown, cooldown);
}

#[rstest]
#[case::exactly_zero(Seconds(0.0), false)]
#[case::negative_zero(Seconds(-0.0), false)]
#[case::positive_tolerance_edge(Seconds(1e-9), false)]
#[case::negative_tolerance_edge(Seconds(-1e-9), false)]
#[case::just_past_tolerance(Seconds(2e-9), true)]
fn cooldown_decisions_time_to_cooldown_boundaries(
    #[case] time_to_cooldown: Seconds,
    #[case] cooldown: bool,
) {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    let decision = decide_with_time(&router, &deployment, &failure, Some(time_to_cooldown));
    assert_eq!(decision.cooldown, cooldown);
}

#[test]
fn cooldown_decisions_counter_uses_the_deployment_class_name() {
    let decision = case_deployment_policy_wins();
    assert_eq!(
        decision.counter,
        Some(AllowedFailsCounter {
            key: "deployment:dep-1:allowed_fails:ContentPolicyViolationError".to_owned(),
            ttl_seconds: Seconds(5.0),
            fails: 2,
        })
    );
}
