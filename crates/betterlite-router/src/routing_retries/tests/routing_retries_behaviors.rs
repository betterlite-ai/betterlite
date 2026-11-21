use std::collections::BTreeMap;

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use serde_json::Number;

use super::{
    GROUP, Scenario, Step, authentication, availability, bad_request, drive, internal, millis,
    mock_switch, rate_limit, stamped,
};
use crate::cooldown_decisions::failure::FailureClass;
use crate::retry_policy::policy::{RetryPolicies, RetryPolicy};
use crate::router_core::headers::{Response, prepare_response};
use crate::routing_retries::headers::install_retry_headers;
use crate::routing_retries::trail::{RetryAttempt, RetryTrail};

fn policy(bad_request: Option<i64>, internal_server: Option<i64>) -> RetryPolicies {
    RetryPolicies {
        global: Some(RetryPolicy {
            bad_request,
            authentication: None,
            timeout: None,
            rate_limit: None,
            content_policy: None,
            internal_server,
            service_unavailable: None,
            not_found: None,
            fallback: None,
        }),
        model_group: BTreeMap::new(),
    }
}

fn with_deployment_retries(mut scenario: Scenario, retries: u32) -> Scenario {
    scenario.deployment_num_retries = Some(retries);
    scenario
}

fn with_mock_rate_limit(mut scenario: Scenario) -> Scenario {
    scenario.request = mock_switch();
    scenario
}

fn with_retry_after(mut scenario: Scenario, seconds: i64) -> Scenario {
    scenario.retry_after = Some(seconds);
    scenario
}

fn with_policies(mut scenario: Scenario, policies: RetryPolicies) -> Scenario {
    scenario.policies = policies;
    scenario
}

fn with_availability(mut scenario: Scenario, seen: Vec<(usize, usize)>) -> Scenario {
    scenario.availability = seen
        .into_iter()
        .map(|(healthy, all)| availability(healthy, all))
        .collect();
    scenario
}

// (answer, provider calls, attempted retries, max retries, backoff millis, random draws)
type Expected = (&'static str, usize, i64, i64, Vec<u64>, usize);

#[rstest]
#[case::first_attempt_answers(Scenario::new(vec![Step::Answer("ok")], 2), ("ok", 1, 0, 2, vec![], 0))]
#[case::rate_limit_then_answer(
    Scenario::new(vec![Step::Fail("a", rate_limit("busy")), Step::Answer("ok")], 2),
    ("ok", 2, 1, 2, vec![500], 1)
)]
#[case::backoff_doubles_per_retry(
    Scenario::new(
        vec![
            Step::Fail("a", internal("one")),
            Step::Fail("a", internal("two")),
            Step::Fail("a", internal("three")),
            Step::Answer("ok"),
        ],
        3,
    ),
    ("ok", 4, 3, 3, vec![500, 1000, 2000], 3)
)]
#[case::healthy_alternative_retries_at_once(
    with_availability(
        Scenario::new(vec![Step::Fail("a", internal("down")), Step::Answer("ok")], 1),
        vec![(2, 2)],
    ),
    ("ok", 2, 1, 1, vec![0], 0)
)]
#[case::deployment_count_replaces_the_router_count(
    with_deployment_retries(
        Scenario::new(vec![Step::Fail("a", internal("down")), Step::Answer("ok")], 0),
        1,
    ),
    ("ok", 2, 1, 1, vec![500], 1)
)]
#[case::error_count_outranks_the_deployment_count(
    with_deployment_retries(
        Scenario::new(
            vec![
                Step::Fail("a", stamped("counted", 2)),
                Step::Fail("a", internal("down")),
                Step::Answer("ok"),
            ],
            0,
        ),
        1,
    ),
    ("ok", 3, 2, 2, vec![500, 1000], 2)
)]
#[case::mock_rate_limit_enters_the_retry_path(
    with_mock_rate_limit(with_deployment_retries(Scenario::new(vec![Step::Answer("ok")], 0), 1)),
    ("ok", 1, 1, 1, vec![500], 1)
)]
#[case::retry_after_header_sets_the_wait(
    with_retry_after(
        Scenario::new(vec![Step::Fail("a", rate_limit("busy")), Step::Answer("ok")], 1),
        5,
    ),
    ("ok", 2, 1, 1, vec![5000], 1)
)]
#[case::policy_retries_a_bad_request(
    with_policies(
        Scenario::new(vec![Step::Fail("a", bad_request("bad")), Step::Answer("ok")], 0),
        policy(Some(1), None),
    ),
    ("ok", 2, 1, 1, vec![500], 1)
)]
#[case::policy_skips_the_check_on_later_errors(
    with_policies(
        Scenario::new(
            vec![
                Step::Fail("a", internal("down")),
                Step::Fail("a", bad_request("bad")),
                Step::Answer("ok"),
            ],
            0,
        ),
        policy(None, Some(2)),
    ),
    ("ok", 3, 2, 2, vec![500, 1000], 2)
)]
#[case::auth_keeps_the_first_deployment_count(
    with_availability(
        Scenario::new(
            vec![
                Step::Fail("a", authentication("key")),
                Step::Fail("b", authentication("key")),
                Step::Answer("ok"),
            ],
            2,
        ),
        vec![(1, 2), (1, 1)],
    ),
    ("ok", 3, 2, 2, vec![0, 0], 0)
)]
#[case::committed_stream_is_never_attempted_again(
    Scenario::new(
        vec![
            Step::Fail("a", rate_limit("busy")),
            Step::Answer("stream"),
            Step::Fail("a", rate_limit("busy")),
        ],
        2,
    ),
    ("stream", 2, 1, 2, vec![500], 1)
)]
#[tokio::test(start_paused = true)]
async fn retried_success(#[case] scenario: Scenario, #[case] expected: Expected) {
    let (answer, calls, attempted, max, backoffs, draws) = expected;
    let driven = drive(scenario).await;
    assert_eq!(driven.retried.outcome.unwrap(), answer);
    assert_eq!(driven.calls.len(), calls);
    assert_eq!(driven.retried.trail.attempted_retries, attempted);
    assert_eq!(driven.retried.trail.max_retries, max);
    assert_eq!(driven.retried.trail.backoffs, millis(&backoffs));
    assert_eq!(driven.draws, draws);
}

#[tokio::test(start_paused = true)]
async fn each_attempt_sees_the_counts_the_metadata_holds() {
    let scenario = Scenario::new(
        vec![
            Step::Fail("a", rate_limit("busy")),
            Step::Fail("a", internal("down")),
            Step::Answer("ok"),
        ],
        2,
    );
    let driven = drive(scenario).await;
    let seen: Vec<_> = driven
        .calls
        .iter()
        .map(|call| (call.attempted_retries, call.max_retries, call.breadcrumbs))
        .collect();
    assert_eq!(seen, vec![(0, 2, 0), (1, 2, 1), (2, 2, 2)]);
    assert_eq!(driven.retried.outcome.unwrap(), "ok");
    let trail = driven.retried.trail;
    assert_eq!(trail.model_group_size, 2);
    assert_eq!(trail.request_retry_count, 2);
    assert_eq!(
        trail.previous_models,
        vec![
            RetryAttempt {
                model_group: GROUP.to_owned(),
                deployment_id: Some("a".to_owned()),
                class: FailureClass::RateLimit,
                status: Some(429),
                attempted_retries: 0,
            },
            RetryAttempt {
                model_group: GROUP.to_owned(),
                deployment_id: Some("a".to_owned()),
                class: FailureClass::InternalServer,
                status: Some(500),
                attempted_retries: 1,
            },
        ]
    );
}

fn trail(attempted_retries: i64, max_retries: i64) -> RetryTrail {
    RetryTrail {
        model_group_size: 1,
        attempted_retries,
        max_retries,
        previous_models: Vec::new(),
        request_retry_count: 0,
        backoffs: Vec::new(),
    }
}

fn headers(entries: &[(&str, i64)]) -> AdditionalHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Number(Number::from(*value))))
        .collect()
}

#[rstest]
#[case::first_success_names_no_maximum(Response::Object(JsonObject::new()), trail(0, 3), &[("x-litellm-attempted-retries", 0)])]
#[case::retried_success_names_both(
    Response::Object(JsonObject::new()),
    trail(2, 3),
    &[("x-litellm-attempted-retries", 2), ("x-litellm-max-retries", 3)]
)]
#[case::stream_carries_them_too(
    Response::Stream,
    trail(1, 1),
    &[("x-litellm-attempted-retries", 1), ("x-litellm-max-retries", 1)]
)]
fn retry_headers_reach_ordinary_and_stream_successes(
    #[case] response: Response,
    #[case] trail: RetryTrail,
    #[case] expected: &[(&str, i64)],
) {
    let mut prepared = prepare_response(response).unwrap();
    install_retry_headers(&mut prepared, &trail);
    let hidden = prepared.hidden_params();
    assert_eq!(
        hidden.get("additional_headers"),
        Some(&Json::Object(headers(expected)))
    );
}
