use betterlite_models::decisions::Json;

use crate::cooldown_decisions::failure::{ErrorStatus, Seconds};
use crate::cooldown_policy::error::CooldownPolicyError;

use super::support::{
    Fixture, attempt, attempt_with, deployment, deployment_with, float, harness, policy,
    rate_limit, ratio_policy, stored_record, text,
};

pub(super) async fn the_fail_count_is_strict_at_the_threshold() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(2_u32));
    let lookup = harness.fixture.lookup();
    let attempt = attempt("a", &failure);
    for expected in [false, false, true] {
        assert_eq!(
            harness
                .service
                .deployment_callback_on_failure(&lookup, &attempt)
                .await
                .unwrap(),
            expected
        );
    }
    assert_eq!(harness.events.recorded().len(), 1);
}

pub(super) async fn zero_allowed_fails_cools_on_the_first_failure() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    assert!(
        harness
            .service
            .deployment_callback_on_failure(&lookup, &attempt("a", &failure))
            .await
            .unwrap()
    );
}

pub(super) async fn a_zero_header_stops_the_cooldown_and_a_tiny_one_keeps_it() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    let zero = attempt_with("a", &failure, Some(Seconds(0.0_f64)), None);
    assert!(
        !harness
            .service
            .deployment_callback_on_failure(&lookup, &zero)
            .await
            .unwrap()
    );
    assert!(stored_record(&harness.shared, "a").is_none());
    let tiny = attempt_with("a", &failure, Some(Seconds(0.5_f64)), None);
    assert!(
        harness
            .service
            .deployment_callback_on_failure(&lookup, &tiny)
            .await
            .unwrap()
    );
    assert_eq!(
        stored_record(&harness.shared, "a").unwrap().cooldown_time,
        0.5_f64
    );
}

pub(super) async fn a_negative_header_falls_to_the_router_default() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    let negative = attempt_with("a", &failure, Some(Seconds(-3.0_f64)), None);
    assert!(
        harness
            .service
            .deployment_callback_on_failure(&lookup, &negative)
            .await
            .unwrap()
    );
    assert_eq!(
        stored_record(&harness.shared, "a").unwrap().cooldown_time,
        5.0_f64
    );
}

pub(super) async fn a_negative_model_info_value_hides_a_positive_params_value() {
    let failure = rate_limit(429);
    let deployment = deployment_with(
        "gpt",
        "a",
        vec![("cooldown_time", float(4.0_f64))],
        vec![("cooldown_time", float(-1.0_f64))],
    );
    let harness = harness(Fixture::new(vec![deployment]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    let attempt = attempt_with("a", &failure, Some(Seconds(9.0_f64)), None);
    assert!(
        harness
            .service
            .deployment_callback_on_failure(&lookup, &attempt)
            .await
            .unwrap()
    );
    assert_eq!(
        stored_record(&harness.shared, "a").unwrap().cooldown_time,
        9.0_f64
    );
}

pub(super) async fn a_text_status_is_cast_and_stored() {
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    let mut padded = rate_limit(429);
    padded.status = ErrorStatus::Text(" 429 ".to_owned());
    assert!(
        harness
            .service
            .deployment_callback_on_failure(&lookup, &attempt("a", &padded))
            .await
            .unwrap()
    );
    assert_eq!(
        stored_record(&harness.shared, "a").unwrap().status_code,
        "429"
    );
    let mut unreadable = rate_limit(429);
    unreadable.status = ErrorStatus::Text("oops".to_owned());
    let harness = harness(Fixture::new(vec![deployment("gpt", "b")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    assert!(
        harness
            .service
            .deployment_callback_on_failure(&lookup, &attempt("b", &unreadable))
            .await
            .unwrap()
    );
    assert_eq!(
        stored_record(&harness.shared, "b").unwrap().status_code,
        "500"
    );
}

pub(super) async fn a_wrong_allowed_fails_type_names_the_field() {
    let failure = rate_limit(429);
    let deployment = deployment_with("gpt", "a", Vec::new(), vec![("allowed_fails", text("2"))]);
    let harness = harness(Fixture::new(vec![deployment]), policy(1_u32));
    let lookup = harness.fixture.lookup();
    let error = harness
        .service
        .deployment_callback_on_failure(&lookup, &attempt("a", &failure))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CooldownPolicyError::DeploymentFact {
            field: "allowed_fails",
            ..
        }
    ));
}

pub(super) async fn a_wrong_policy_value_type_names_the_policy_key() {
    let failure = rate_limit(429);
    let deployment = deployment_with(
        "gpt",
        "a",
        Vec::new(),
        vec![(
            "allowed_fails_policy",
            Json::Object(
                [("RateLimitErrorAllowedFails".to_owned(), text("one"))]
                    .into_iter()
                    .collect(),
            ),
        )],
    );
    let harness = harness(Fixture::new(vec![deployment]), policy(1_u32));
    let lookup = harness.fixture.lookup();
    let error = harness
        .service
        .deployment_callback_on_failure(&lookup, &attempt("a", &failure))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CooldownPolicyError::DeploymentFact {
            field: "RateLimitErrorAllowedFails",
            ..
        }
    ));
}

pub(super) async fn a_wrong_cooldown_time_type_names_the_field() {
    let failure = rate_limit(429);
    let deployment = deployment_with(
        "gpt",
        "a",
        vec![("cooldown_time", text("soon"))],
        Vec::new(),
    );
    let harness = harness(Fixture::new(vec![deployment]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    let error = harness
        .service
        .deployment_callback_on_failure(&lookup, &attempt("a", &failure))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CooldownPolicyError::DeploymentFact {
            field: "cooldown_time",
            ..
        }
    ));
}

pub(super) async fn a_non_policy_value_reads_as_no_policy() {
    let failure = rate_limit(429);
    let deployment = deployment_with(
        "gpt",
        "a",
        Vec::new(),
        vec![("allowed_fails_policy", text("nope"))],
    );
    let harness = harness(Fixture::new(vec![deployment]), policy(1_u32));
    let lookup = harness.fixture.lookup();
    assert!(
        !harness
            .service
            .deployment_callback_on_failure(&lookup, &attempt("a", &failure))
            .await
            .unwrap()
    );
}

pub(super) async fn the_ratio_minimum_requests_boundary_sits_at_five() {
    let failure = rate_limit(500);
    let harness = harness(
        Fixture::new(vec![deployment("gpt", "a"), deployment("gpt", "b")]),
        ratio_policy(),
    );
    let lookup = harness.fixture.lookup();
    let attempt = attempt("a", &failure);
    for _ in 0..4_i32 {
        assert!(
            !harness
                .service
                .deployment_callback_on_failure(&lookup, &attempt)
                .await
                .unwrap()
        );
    }
    assert!(
        harness
            .service
            .deployment_callback_on_failure(&lookup, &attempt)
            .await
            .unwrap()
    );
}

pub(super) async fn a_team_with_one_routable_sibling_keeps_the_net() {
    let failure = rate_limit(429);
    let team = vec![
        ("team_id", text("probe-team")),
        ("team_public_model_name", text("probe-public")),
    ];
    let blocked_team = vec![
        ("team_id", text("probe-team")),
        ("team_public_model_name", text("probe-public")),
        ("blocked", Json::Bool(true)),
    ];
    let fixture = Fixture::new(vec![
        deployment_with("gpt", "a", Vec::new(), team.clone()),
        deployment_with("other", "b", Vec::new(), blocked_team.clone()),
        deployment_with("third", "c", Vec::new(), blocked_team),
    ]);
    let harness = harness(fixture, ratio_policy());
    let lookup = harness.fixture.lookup();
    assert!(
        !harness
            .service
            .deployment_callback_on_failure(&lookup, &attempt("a", &failure))
            .await
            .unwrap()
    );
}
