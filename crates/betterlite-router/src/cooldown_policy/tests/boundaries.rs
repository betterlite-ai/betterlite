use std::iter::once;

use betterlite_models::decisions::Json;

use crate::cooldown_decisions::failure::{ErrorStatus, Seconds};
use crate::cooldown_policy::error::CooldownPolicyError;
use crate::cooldown_policy::service::CooldownSource;
use crate::deployment_identity::spec::DeploymentSpec;

use super::support::{
    self, Fixture, attempt, attempt_with, deployment, deployment_with, fallback, float, harness,
    policy, rate_limit, ratio_policy, stored_record, text,
};

pub(super) async fn the_fail_count_is_strict_at_the_threshold() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(2_u32));
    let attempt = attempt("a", &failure);
    for expected in [false, false, true] {
        assert_eq!(
            harness
                .cooling(CooldownSource::Callback(&attempt))
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
    assert!(
        harness
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
}

pub(super) async fn a_zero_header_stops_the_cooldown_and_a_tiny_one_keeps_it() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let zero = attempt_with("a", &failure, Some(Seconds(0.0_f64)), None);
    assert!(
        !harness
            .cooling(CooldownSource::Callback(&zero))
            .await
            .unwrap()
    );
    assert!(stored_record(&harness.shared, "a").is_none());
    let tiny = attempt_with("a", &failure, Some(Seconds(0.5_f64)), None);
    assert!(
        harness
            .cooling(CooldownSource::Callback(&tiny))
            .await
            .unwrap()
    );
    let record = stored_record(&harness.shared, "a").unwrap();
    assert_eq!(record.cooldown_time.to_bits(), 0.5_f64.to_bits());
}

pub(super) async fn a_negative_header_falls_to_the_router_default() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let negative = attempt_with("a", &failure, Some(Seconds(-3.0_f64)), None);
    assert!(
        harness
            .cooling(CooldownSource::Callback(&negative))
            .await
            .unwrap()
    );
    let record = stored_record(&harness.shared, "a").unwrap();
    assert_eq!(record.cooldown_time.to_bits(), 5.0_f64.to_bits());
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
    let attempt = attempt_with("a", &failure, Some(Seconds(9.0_f64)), None);
    assert!(
        harness
            .cooling(CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    let record = stored_record(&harness.shared, "a").unwrap();
    assert_eq!(record.cooldown_time.to_bits(), 9.0_f64.to_bits());
}

pub(super) async fn a_text_status_is_cast_and_stored() {
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let mut padded = rate_limit(429);
    padded.status = ErrorStatus::Text(" 429 ".to_owned());
    assert!(
        harness
            .cooling(CooldownSource::Callback(&attempt("a", &padded)))
            .await
            .unwrap()
    );
    let stored = stored_record(&harness.shared, "a").unwrap();
    assert_eq!(stored.status_code, "429");
    let mut unreadable = rate_limit(429);
    unreadable.status = ErrorStatus::Text("oops".to_owned());
    let unreadable_harness =
        support::harness(Fixture::new(vec![deployment("gpt", "b")]), policy(0_u32));
    assert!(
        unreadable_harness
            .cooling(CooldownSource::Callback(&attempt("b", &unreadable)))
            .await
            .unwrap()
    );
    let stored = stored_record(&unreadable_harness.shared, "b").unwrap();
    assert_eq!(stored.status_code, "500");
}

pub(super) async fn deployment_value_types_are_classified() {
    let failure = rate_limit(429);
    for (deployment_id, deployment, field) in typed_deployments() {
        let harness = harness(Fixture::new(vec![deployment]), policy(1_u32));
        let error = harness
            .cooling(CooldownSource::Callback(&attempt(deployment_id, &failure)))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            CooldownPolicyError::DeploymentFact { field: named, .. } if named == field
        ));
    }
    let non_policy = deployment_with(
        "gpt",
        "a",
        Vec::new(),
        vec![("allowed_fails_policy", text("nope"))],
    );
    let harness = harness(Fixture::new(vec![non_policy]), policy(1_u32));
    assert!(
        !harness
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
}

pub(super) async fn the_fallback_trigger_swallows_a_classification_error() {
    let failure = rate_limit(429);
    let broken = deployment_with("gpt", "d", Vec::new(), vec![("allowed_fails", text("2"))]);
    let harness = harness(Fixture::new(vec![broken]), policy(0_u32));
    let fallback_failure = fallback(Some("d"), &failure);
    assert!(!harness.fallback_trigger(&fallback_failure).await);
}

fn typed_deployments() -> [(&'static str, DeploymentSpec, &'static str); 3] {
    [
        (
            "a",
            deployment_with("gpt", "a", Vec::new(), vec![("allowed_fails", text("2"))]),
            "allowed_fails",
        ),
        (
            "b",
            deployment_with(
                "gpt",
                "b",
                Vec::new(),
                vec![(
                    "allowed_fails_policy",
                    Json::Object(
                        once(("RateLimitErrorAllowedFails".to_owned(), text("one"))).collect(),
                    ),
                )],
            ),
            "RateLimitErrorAllowedFails",
        ),
        (
            "c",
            deployment_with(
                "gpt",
                "c",
                vec![("cooldown_time", text("soon"))],
                Vec::new(),
            ),
            "cooldown_time",
        ),
    ]
}

pub(super) async fn the_ratio_minimum_requests_boundary_sits_at_five() {
    let failure = rate_limit(500);
    let harness = harness(
        Fixture::new(vec![deployment("gpt", "a"), deployment("gpt", "b")]),
        ratio_policy(),
    );
    let attempt = attempt("a", &failure);
    for _ in 0_i32..4_i32 {
        assert!(
            !harness
                .cooling(CooldownSource::Callback(&attempt))
                .await
                .unwrap()
        );
    }
    assert!(
        harness
            .cooling(CooldownSource::Callback(&attempt))
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
    assert!(
        !harness
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
}
