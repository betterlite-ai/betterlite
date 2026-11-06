#![cfg(test)]

use rstest::rstest;

use super::decision::{
    AllowedFailsCounter, CooldownDecision, CooldownRequest, DeploymentFacts, MinuteSamples, decide,
};
use super::failure::{ErrorStatus, FailureClass, FailureFacts, Seconds, TimeoutFacts};
use super::policy::{AllowedFailsPolicy, CooldownPolicy, FailureThresholds};

#[derive(Debug, Clone, Copy)]
enum PolicyEntry {
    ContentPolicyViolation,
    BadRequest,
    Authentication,
    Timeout,
    RateLimit,
    InternalServer,
    ServiceUnavailable,
    BadGateway,
    NotFound,
}

fn policy_with(entry: PolicyEntry, value: u32) -> AllowedFailsPolicy {
    let mut policy = AllowedFailsPolicy::default();
    match entry {
        PolicyEntry::ContentPolicyViolation => {
            policy.content_policy_violation_error = Some(value);
        }
        PolicyEntry::BadRequest => {
            policy.bad_request_error = Some(value);
        }
        PolicyEntry::Authentication => {
            policy.authentication_error = Some(value);
        }
        PolicyEntry::Timeout => {
            policy.timeout_error = Some(value);
        }
        PolicyEntry::RateLimit => {
            policy.rate_limit_error = Some(value);
        }
        PolicyEntry::InternalServer => {
            policy.internal_server_error = Some(value);
        }
        PolicyEntry::ServiceUnavailable => {
            policy.service_unavailable_error = Some(value);
        }
        PolicyEntry::BadGateway => {
            policy.bad_gateway_error = Some(value);
        }
        PolicyEntry::NotFound => {
            policy.not_found_error = Some(value);
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
        model_info_cooldown_time: None,
        litellm_params_cooldown_time: None,
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
    router.allowed_fails_policy = Some(policy_with(PolicyEntry::BadRequest, 100));
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::ContentPolicyViolation, 1));
    let failure = failure(FailureClass::ContentPolicyViolation, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 1)
}

fn case_deployment_policy_miss_falls_back_to_router() -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails_policy = Some(policy_with(PolicyEntry::RateLimit, 2));
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::BadGateway, 9));
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 2)
}

fn case_router_policy_applies() -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails = 10;
    router.allowed_fails_policy = Some(policy_with(PolicyEntry::RateLimit, 2));
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

fn case_cooldown_time(
    model_info: Option<Seconds>,
    litellm_params: Option<Seconds>,
) -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::ContentPolicyViolation, 1));
    deployment.model_info_cooldown_time = model_info;
    deployment.litellm_params_cooldown_time = litellm_params;
    let failure = failure(FailureClass::ContentPolicyViolation, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 1)
}

fn case_zero_cooldown_time() -> CooldownDecision {
    let mut router = policy();
    router.cooldown_time = Seconds(0.0);
    router.allowed_fails = 0;
    router.default_allowed_fails = 0;
    router.allowed_fails_policy = Some(policy_with(PolicyEntry::RateLimit, 0));
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(500));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_router_policy_does_not_override_400() -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails_policy = Some(policy_with(PolicyEntry::BadRequest, 0));
    let deployment = deployment();
    let failure = failure(FailureClass::BadRequest, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_content_policy_falls_through_to_bad_request() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::BadRequest, 0));
    let failure = failure(FailureClass::ContentPolicyViolation, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_mid_stream_matches_service_unavailable() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::ServiceUnavailable, 0));
    let failure = failure(FailureClass::MidStreamFallback, ErrorStatus::Code(503));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_context_window_matches_bad_request() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::BadRequest, 0));
    let failure = failure(FailureClass::ContextWindowExceeded, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_unknown_deployment() -> CooldownDecision {
    let router = policy();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, None, &failure, minute(0, 0), 0)
}

fn case_bad_request_policy() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::BadRequest, 0));
    let failure = failure(FailureClass::BadRequest, ErrorStatus::Code(400));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_single_group_with_alternatives() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.model_group_size = 1;
    deployment.has_alternatives = true;
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
}

fn case_policy_beats_generic_on_single_group() -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.model_group_size = 1;
    deployment.allowed_fails = Some(5);
    deployment.allowed_fails_policy = Some(policy_with(PolicyEntry::RateLimit, 0));
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), 0)
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
    case_cooldown_time(Some(Seconds(2.5)), None),
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
#[case::bad_request_policy(
    case_bad_request_policy(),
    true,
    Some("deployment:dep-1:allowed_fails:BadRequestError"),
    Some(1)
)]
#[case::single_group_with_alternatives(case_single_group_with_alternatives(), true, None, None)]
#[case::policy_beats_generic_on_single_group(
    case_policy_beats_generic_on_single_group(),
    true,
    Some("deployment:dep-1:allowed_fails:RateLimitError"),
    Some(1)
)]
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
#[case::deployment_cooldown_time_overrides_router(
    case_cooldown_time(Some(Seconds(2.5)), None),
    Seconds(2.5)
)]
#[case::litellm_params_cooldown_time(case_cooldown_time(None, Some(Seconds(1.25))), Seconds(1.25))]
#[case::model_info_cooldown_time_wins(
    case_cooldown_time(Some(Seconds(2.5)), Some(Seconds(1.25))),
    Seconds(2.5)
)]
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

fn traffic_status_decision(status: ErrorStatus, successes: u64, failures: u64) -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::BadRequest, status);
    run(
        &router,
        Some(&deployment),
        &failure,
        minute(successes, failures),
        0,
    )
}

fn counter_decision(before: u64) -> CooldownDecision {
    let mut router = policy();
    router.allowed_fails = 3;
    router.default_allowed_fails = 0;
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    run(&router, Some(&deployment), &failure, minute(0, 0), before)
}

fn ratio_decision(successes: u64, failures: u64, model_group_size: u32) -> CooldownDecision {
    let router = policy();
    let mut deployment = deployment();
    deployment.model_group_size = model_group_size;
    let failure = failure(FailureClass::InternalServer, ErrorStatus::Code(500));
    run(
        &router,
        Some(&deployment),
        &failure,
        minute(successes, failures),
        0,
    )
}

fn time_decision(time_to_cooldown: Seconds) -> CooldownDecision {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Code(429));
    decide_with_time(&router, &deployment, &failure, Some(time_to_cooldown))
}

#[rstest]
#[case::status_below_400(status_decision(ErrorStatus::Code(399)), true, None)]
#[case::status_bad_request(status_decision(ErrorStatus::Code(400)), false, None)]
#[case::status_unauthorized(status_decision(ErrorStatus::Code(401)), true, None)]
#[case::status_forbidden(status_decision(ErrorStatus::Code(403)), false, None)]
#[case::status_not_found(status_decision(ErrorStatus::Code(404)), true, None)]
#[case::status_request_timeout(status_decision(ErrorStatus::Code(408)), false, None)]
#[case::status_conflict(status_decision(ErrorStatus::Code(409)), false, None)]
#[case::status_unprocessable(status_decision(ErrorStatus::Code(422)), false, None)]
#[case::status_rate_limit(status_decision(ErrorStatus::Code(429)), true, None)]
#[case::status_last_client_error(status_decision(ErrorStatus::Code(499)), false, None)]
#[case::status_server_error(status_decision(ErrorStatus::Code(500)), false, None)]
#[case::status_top_server_error(status_decision(ErrorStatus::Code(599)), false, None)]
#[case::text_empty_status(
    traffic_status_decision(ErrorStatus::Text(String::new()), 0, 0),
    false,
    None
)]
#[case::text_empty_status_with_traffic(
    traffic_status_decision(ErrorStatus::Text(String::new()), 0, 5),
    false,
    None
)]
#[case::text_unparsable_status(
    traffic_status_decision(ErrorStatus::Text("abc".to_owned()), 0, 0),
    false,
    None
)]
#[case::text_unparsable_status_with_traffic(
    traffic_status_decision(ErrorStatus::Text("abc".to_owned()), 0, 5),
    true,
    None
)]
#[case::text_unauthorized(
    traffic_status_decision(ErrorStatus::Text("401".to_owned()), 0, 0),
    true,
    None
)]
#[case::text_rate_limit(
    traffic_status_decision(ErrorStatus::Text("429".to_owned()), 0, 0),
    true,
    None
)]
#[case::text_padded_server_error(
    traffic_status_decision(ErrorStatus::Text(" 500 ".to_owned()), 0, 0),
    false,
    None
)]
#[case::counter_below_threshold(counter_decision(2), false, Some(3))]
#[case::counter_at_threshold(counter_decision(3), true, Some(4))]
#[case::counter_saturated(counter_decision(u64::MAX), true, Some(u64::MAX))]
#[case::ratio_below_minimum(ratio_decision(0, 4, 2), false, None)]
#[case::ratio_at_minimum(ratio_decision(0, 5, 2), true, None)]
#[case::ratio_just_above_half(ratio_decision(2, 3, 2), true, None)]
#[case::ratio_exactly_half(ratio_decision(3, 3, 2), false, None)]
#[case::single_group_traffic_below(ratio_decision(0, 999, 1), false, None)]
#[case::single_group_traffic_at_boundary(ratio_decision(0, 1000, 1), true, None)]
#[case::high_traffic_with_successes(ratio_decision(600, 400, 2), false, None)]
#[case::saturated_total(ratio_decision(u64::MAX, u64::MAX, 2), true, None)]
#[case::time_zero(time_decision(Seconds(0.0)), false, None)]
#[case::time_negative_zero(time_decision(Seconds(-0.0)), false, None)]
#[case::time_positive_edge(time_decision(Seconds(1e-9)), false, None)]
#[case::time_negative_edge(time_decision(Seconds(-1e-9)), false, None)]
#[case::time_past_edge(time_decision(Seconds(2e-9)), true, None)]
fn cooldown_decisions_boundaries(
    #[case] decision: CooldownDecision,
    #[case] cooldown: bool,
    #[case] fails: Option<u64>,
) {
    assert_eq!(decision.cooldown, cooldown);
    assert_eq!(counter_fails(&decision), fails);
    if fails.is_none() {
        assert_eq!(decision.counter, None);
    }
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

#[rstest]
#[case::content_policy_violation(
    PolicyEntry::ContentPolicyViolation,
    FailureClass::ContentPolicyViolation,
    ErrorStatus::Code(400),
    "ContentPolicyViolationError"
)]
#[case::bad_request(
    PolicyEntry::BadRequest,
    FailureClass::BadRequest,
    ErrorStatus::Code(400),
    "BadRequestError"
)]
#[case::authentication(
    PolicyEntry::Authentication,
    FailureClass::Authentication,
    ErrorStatus::Code(401),
    "AuthenticationError"
)]
#[case::timeout(
    PolicyEntry::Timeout,
    FailureClass::Timeout,
    ErrorStatus::Code(408),
    "Timeout"
)]
#[case::rate_limit(
    PolicyEntry::RateLimit,
    FailureClass::RateLimit,
    ErrorStatus::Code(429),
    "RateLimitError"
)]
#[case::internal_server(
    PolicyEntry::InternalServer,
    FailureClass::InternalServer,
    ErrorStatus::Code(500),
    "InternalServerError"
)]
#[case::service_unavailable(
    PolicyEntry::ServiceUnavailable,
    FailureClass::ServiceUnavailable,
    ErrorStatus::Code(503),
    "ServiceUnavailableError"
)]
#[case::bad_gateway(
    PolicyEntry::BadGateway,
    FailureClass::BadGateway,
    ErrorStatus::Code(502),
    "BadGatewayError"
)]
#[case::not_found(
    PolicyEntry::NotFound,
    FailureClass::NotFound,
    ErrorStatus::Code(404),
    "NotFoundError"
)]
fn cooldown_decisions_policy_fields(
    #[case] entry: PolicyEntry,
    #[case] class: FailureClass,
    #[case] status: ErrorStatus,
    #[case] class_name: &str,
) {
    let router = policy();
    let mut deployment = deployment();
    deployment.allowed_fails_policy = Some(policy_with(entry, 0));
    let failure = failure(class, status);
    let decision = run(&router, Some(&deployment), &failure, minute(0, 0), 0);
    assert!(decision.cooldown);
    assert_eq!(
        counter_key(&decision),
        Some(format!("deployment:dep-1:allowed_fails:{class_name}").as_str())
    );
}

#[rstest]
#[case::authentication(FailureClass::Authentication, "AuthenticationError")]
#[case::not_found(FailureClass::NotFound, "NotFoundError")]
#[case::permission_denied(FailureClass::PermissionDenied, "PermissionDeniedError")]
#[case::bad_request(FailureClass::BadRequest, "BadRequestError")]
#[case::unprocessable_entity(FailureClass::UnprocessableEntity, "UnprocessableEntityError")]
#[case::timeout(FailureClass::Timeout, "Timeout")]
#[case::rate_limit(FailureClass::RateLimit, "RateLimitError")]
#[case::budget_exceeded(FailureClass::BudgetExceeded, "BudgetExceededError")]
#[case::context_window_exceeded(FailureClass::ContextWindowExceeded, "ContextWindowExceededError")]
#[case::content_policy_violation(
    FailureClass::ContentPolicyViolation,
    "ContentPolicyViolationError"
)]
#[case::service_unavailable(FailureClass::ServiceUnavailable, "ServiceUnavailableError")]
#[case::bad_gateway(FailureClass::BadGateway, "BadGatewayError")]
#[case::internal_server(FailureClass::InternalServer, "InternalServerError")]
#[case::api(FailureClass::Api, "APIError")]
#[case::api_connection(FailureClass::ApiConnection, "APIConnectionError")]
#[case::api_response_validation(FailureClass::ApiResponseValidation, "APIResponseValidationError")]
#[case::json_schema_validation(FailureClass::JsonSchemaValidation, "JSONSchemaValidationError")]
#[case::unsupported_params(FailureClass::UnsupportedParams, "UnsupportedParamsError")]
#[case::guardrail_raised(FailureClass::GuardrailRaised, "GuardrailRaisedException")]
#[case::mid_stream_fallback(FailureClass::MidStreamFallback, "MidStreamFallbackError")]
#[case::unclassified(FailureClass::Unclassified, "Exception")]
fn cooldown_decisions_exception_class_names(#[case] class: FailureClass, #[case] class_name: &str) {
    assert_eq!(super::failure::exception_class_name(class), class_name);
}

#[rstest]
#[case::below_400(399, false)]
#[case::client_error(400, false)]
#[case::not_found(404, false)]
#[case::request_timeout(408, true)]
#[case::conflict(409, true)]
#[case::rate_limit(429, true)]
#[case::internal_error(500, true)]
#[case::top_server_error(599, true)]
fn cooldown_decisions_retry_boundaries(#[case] code: i64, #[case] retry: bool) {
    assert_eq!(super::decision::should_retry(code), retry);
}

#[test]
fn cooldown_decisions_empty_status_with_traffic_stays_up() {
    let router = policy();
    let deployment = deployment();
    let failure = failure(FailureClass::BadRequest, ErrorStatus::Text(String::new()));
    let decision = run(&router, Some(&deployment), &failure, minute(0, 5), 0);
    assert!(!decision.cooldown);
}

#[test]
fn cooldown_decisions_unparsable_status_with_traffic_cools_down() {
    let router = policy();
    let deployment = deployment();
    let failure = failure(
        FailureClass::BadRequest,
        ErrorStatus::Text("abc".to_owned()),
    );
    let decision = run(&router, Some(&deployment), &failure, minute(0, 5), 0);
    assert!(decision.cooldown);
}

#[test]
fn cooldown_decisions_zero_traffic_threshold_needs_a_failure() {
    let mut router = policy();
    router.thresholds.single_deployment_traffic = 0;
    let deployment = deployment();
    let failure = failure(FailureClass::InternalServer, ErrorStatus::Code(500));
    let decision = run(&router, Some(&deployment), &failure, minute(0, 0), 0);
    assert!(!decision.cooldown);
}
