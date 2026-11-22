use std::iter::once;
use std::sync::Arc;

use betterlite_models::decisions::Json;

use crate::cooldown_decisions::failure::{
    ErrorStatus, FailureClass, FailureFacts, Seconds, TimeoutFacts,
};
use crate::cooldown_policy::service::{
    CooldownAttempt, CooldownCall, CooldownSource, FallbackFailure,
};
use crate::routing_read_batch::{RequestReads, RoutingReadBatch};
use crate::routing_state_store::test_support::FakeRedis;

use super::support::{
    self, FailingObserver, Fixture, Harness, attempt, attempt_with, deployment, deployment_with,
    failure, fallback, float, harness, harness_sharing, harness_with, number, policy, rate_limit,
    ratio_policy, stored_record, text,
};

pub(super) async fn strict_threshold_and_one_event() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(1_u32));
    let attempt = attempt("a", &failure);
    assert!(
        !harness
            .cooling(CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    assert!(
        harness
            .cooling(CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    let record = stored_record(&harness.shared, "a").unwrap();
    assert_eq!(record.status_code, "429");
    assert_eq!(record.exception_received, "probe failure");
    assert_eq!(record.timestamp.to_bits(), 1000.0_f64.to_bits());
    assert_eq!(record.cooldown_time.to_bits(), 5.0_f64.to_bits());
    assert_eq!(harness.events.recorded().len(), 1);
}

pub(super) async fn disabled_cooldowns_count_the_failure_but_never_cool() {
    let mut settings = policy(0_u32);
    settings.disable_cooldowns = true;
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), settings);
    let attempt = attempt("a", &failure);
    assert!(
        !harness
            .cooling(CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    assert!(stored_record(&harness.shared, "a").is_none());
    assert!(harness.events.recorded().is_empty());
    assert_eq!(harness.service.counters.failures_for_minute("a"), 1);
}

pub(super) async fn a_failing_observer_does_not_mask_the_cooldown() {
    let failure = rate_limit(429);
    let harness = harness_with(
        Fixture::new(vec![deployment("gpt", "a")]),
        policy(0_u32),
        vec![Arc::new(FailingObserver)],
    );
    let cooled = harness
        .cooling(CooldownSource::Callback(&attempt("a", &failure)))
        .await
        .unwrap();
    assert!(cooled);
    assert!(stored_record(&harness.shared, "a").is_some());
    assert_eq!(harness.events.recorded().len(), 1);
}

pub(super) async fn fallback_cools_the_server_stamped_deployment() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    assert!(
        harness
            .fallback_trigger(&fallback(Some("a"), &failure))
            .await
    );
    assert_eq!(harness.events.recorded().len(), 1);
    assert_eq!(harness.service.counters.failures_for_minute("a"), 1);
}

pub(super) async fn generic_404_and_caller_timeout_never_cool() {
    let not_found = rate_limit(404);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let generic = FallbackFailure {
        original_generic_function: true,
        ..fallback(Some("a"), &not_found)
    };
    assert!(!fallback_cools(&harness, &generic).await);
    // a background response cost poll's 404 says nothing about deployment health either
    let poll = FailureFacts {
        background_response_cost_poll: true,
        ..rate_limit(404)
    };
    assert!(!callback_cools(&harness, "a", &poll, None).await);
    // the 404 guard covers only the generic calls a caller-supplied id can 404 on its own
    let direct = support::harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    assert!(fallback_cools(&direct, &fallback(Some("a"), &not_found)).await);
    let timed_out = FailureFacts {
        caller_timeout: Some(TimeoutFacts {
            client_side_timeout: true,
            timeout_seconds: Some(Seconds(1.0_f64)),
            elapsed_seconds: Some(Seconds(2.0_f64)),
        }),
        ..failure(FailureClass::Timeout, ErrorStatus::Code(408))
    };
    assert!(!callback_cools(&harness, "a", &timed_out, None).await);
    // the fallback path applies the same caller-timeout guard before it counts anything
    assert!(!fallback_cools(&harness, &fallback(Some("a"), &timed_out)).await);
    assert_eq!(harness.service.counters.failures_for_minute("a"), 0);
    assert!(harness.events.recorded().is_empty());
}

pub(super) async fn deployment_cooldown_times_beat_the_header() {
    let failure = rate_limit(429);
    let both = deployment_with(
        "gpt",
        "a",
        vec![("cooldown_time", float(4.0_f64))],
        vec![("cooldown_time", float(3.0_f64))],
    );
    let harness = harness(Fixture::new(vec![both]), policy(0_u32));
    assert!(callback_cools(&harness, "a", &failure, Some(Seconds(9.0_f64))).await);
    let record = stored_record(&harness.shared, "a").unwrap();
    assert_eq!(record.cooldown_time.to_bits(), 3.0_f64.to_bits());
    let params = deployment_with(
        "gpt",
        "b",
        vec![("cooldown_time", float(4.0_f64))],
        Vec::new(),
    );
    let params_harness = support::harness(Fixture::new(vec![params]), policy(0_u32));
    assert!(callback_cools(&params_harness, "b", &failure, Some(Seconds(9.0_f64))).await);
    let record = stored_record(&params_harness.shared, "b").unwrap();
    assert_eq!(record.cooldown_time.to_bits(), 4.0_f64.to_bits());
}

pub(super) async fn event_fields_come_from_the_deployment() {
    let failure = rate_limit(429);
    let deployment = deployment_with(
        "gpt",
        "a",
        vec![
            ("api_base", text("https://probe.example/v1")),
            ("custom_llm_provider", text("azure")),
        ],
        Vec::new(),
    );
    let harness = harness(Fixture::new(vec![deployment]), policy(0_u32));
    assert!(
        harness
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
    let events = harness.events.recorded();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].model_name, "gpt");
    assert_eq!(events[0].model_id, "a");
    assert_eq!(events[0].api_base, "https://probe.example/v1");
    assert_eq!(events[0].provider, "azure");
    assert_eq!(events[0].exception_status, "429");
}

pub(super) async fn base_url_alias_and_openai_default_feed_the_event() {
    let failure = rate_limit(429);
    let aliased = deployment_with(
        "gpt",
        "a",
        vec![("base_url", text("https://alias.example"))],
        Vec::new(),
    );
    let harness = harness(Fixture::new(vec![aliased]), policy(0_u32));
    assert!(callback_cools(&harness, "a", &failure, None).await);
    let events = harness.events.recorded();
    assert_eq!(events[0].api_base, "https://alias.example");
    assert_eq!(events[0].provider, "openai");
    let plain = support::harness(Fixture::new(vec![deployment("gpt", "b")]), policy(0_u32));
    assert!(callback_cools(&plain, "b", &failure, None).await);
    assert_eq!(
        plain.events.recorded()[0].api_base,
        "https://api.openai.com"
    );
}

pub(super) async fn alternatives_lift_the_single_deployment_net() {
    let failure = rate_limit(429);
    let grouped = Fixture::new(vec![deployment("gpt", "a"), deployment("other", "b")])
        .with_group("alt-group", &["gpt", "other"]);
    let grouped_attempt = attempt_with("a", &failure, None, Some("alt-group"));
    assert!(ratio_cools(grouped, &grouped_attempt).await);
    let alone = Fixture::new(vec![deployment("gpt", "a")]);
    assert!(!ratio_cools(alone, &attempt("a", &failure)).await);
    let team = vec![
        ("team_id", text("probe-team")),
        ("team_public_model_name", text("probe-public")),
    ];
    let team_fixture = Fixture::new(vec![
        deployment_with("gpt", "a", Vec::new(), team.clone()),
        deployment_with("other", "b", Vec::new(), team),
    ]);
    assert!(ratio_cools(team_fixture, &attempt("a", &failure)).await);
    let blocked = vec![
        ("team_id", text("probe-team")),
        ("team_public_model_name", text("probe-public")),
        ("blocked", Json::Bool(true)),
    ];
    let blocked_fixture = Fixture::new(vec![
        deployment_with("gpt", "a", Vec::new(), blocked[..2].to_vec()),
        deployment_with("other", "b", Vec::new(), blocked),
    ]);
    assert!(!ratio_cools(blocked_fixture, &attempt("a", &failure)).await);
}

pub(super) async fn provider_default_deployments_never_cool() {
    let failure = rate_limit(429);
    let fixture = Fixture::new(vec![deployment("gpt", "a")]).with_provider_default("a");
    let harness = harness(fixture, policy(0_u32));
    assert!(!callback_cools(&harness, "a", &failure, None).await);
    let ordinary = support::harness(Fixture::new(vec![deployment("gpt", "b")]), policy(0_u32));
    assert!(callback_cools(&ordinary, "b", &failure, None).await);
}

pub(super) async fn debug_info_lists_only_the_cooling_deployment() {
    let failure = rate_limit(429);
    let harness = harness(
        Fixture::new(vec![deployment("gpt", "a"), deployment("gpt", "b")]),
        policy(0_u32),
    );
    assert!(
        harness
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
    let model_ids = vec!["a".to_owned(), "b".to_owned()];
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    let cooling = harness
        .service
        .cooldown_debug_info(&mut batch, &mut request, &model_ids)
        .await
        .unwrap();
    assert_eq!(cooling.len(), 1);
    assert_eq!(cooling[0].model_id, "a");
    assert_eq!(cooling[0].record.status_code, "429");
    let mut ids_batch = RoutingReadBatch::default();
    let mut ids_request = RequestReads::default();
    let ids = harness
        .service
        .cooling_deployment_ids(&mut ids_batch, &mut ids_request, &model_ids)
        .await
        .unwrap();
    assert_eq!(ids, vec!["a".to_owned()]);
}

pub(super) async fn advisor_failures_skip_both_paths() {
    let mut failure = rate_limit(429);
    failure.advisor_orchestration = true;
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    assert!(
        !harness
            .fallback_trigger(&fallback(Some("a"), &failure))
            .await
    );
    assert!(
        !harness
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
    assert_eq!(harness.service.counters.failures_for_minute("a"), 0);
}

pub(super) async fn an_explicit_policy_cools_without_a_status() {
    let deployment = deployment_with(
        "gpt",
        "a",
        Vec::new(),
        vec![("allowed_fails_policy", policy_json(0_u32))],
    );
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Text(String::new()));
    let harness = harness(Fixture::new(vec![deployment]), policy(1_u32));
    assert!(callback_cools(&harness, "a", &failure, None).await);
    // the record carries cast_exception_status_to_int's 500, while the event carries the raw status
    assert_eq!(
        stored_record(&harness.shared, "a").unwrap().status_code,
        "500"
    );
    assert_eq!(harness.events.recorded()[0].exception_status, "");
    let counted = deployment_with(
        "gpt",
        "b",
        Vec::new(),
        vec![("allowed_fails_policy", policy_json(1_u32))],
    );
    let counted_harness = support::harness(Fixture::new(vec![counted]), policy(5_u32));
    assert!(!callback_cools(&counted_harness, "b", &failure, None).await);
}

pub(super) async fn a_team_id_without_a_public_name_does_not_lift_the_net() {
    let failure = rate_limit(429);
    let fixture = Fixture::new(vec![
        deployment_with(
            "gpt",
            "a",
            Vec::new(),
            vec![("team_id", text("probe-team"))],
        ),
        deployment("other", "b"),
    ]);
    let harness = harness(fixture, ratio_policy());
    assert!(
        !harness
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
}

pub(super) async fn shared_fail_counts_answer_across_workers() {
    let server = FakeRedis::try_start().await.unwrap();
    let failure = rate_limit(429);
    let worker_a = harness_sharing(
        Fixture::new(vec![deployment("gpt", "a")]),
        policy(1_u32),
        &server,
    );
    assert!(
        !worker_a
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
    // worker A leaves the shared count at 1, so only a shared read of 2 cools worker B
    let worker_b = harness_sharing(
        Fixture::new(vec![deployment("gpt", "a")]),
        policy(1_u32),
        &server,
    );
    assert!(
        worker_b
            .cooling(CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
    assert_eq!(worker_b.service.counters.failures_for_minute("a"), 1);
}

pub(super) async fn unknown_deployments_never_cool() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let missing = CooldownCall {
        deployment_id: "missing",
        failure: &failure,
        exception: "probe failure",
        time_to_cooldown: None,
        requested_model_group: None,
    };
    let direct = harness
        .cooling(CooldownSource::Direct(&missing))
        .await
        .unwrap();
    let callback = harness
        .cooling(CooldownSource::Callback(&attempt("missing", &failure)))
        .await
        .unwrap();
    let mut batch = RoutingReadBatch::default();
    let mut request = RequestReads::default();
    let cooling = harness
        .service
        .cooldown_debug_info(&mut batch, &mut request, &["missing".to_owned()])
        .await
        .unwrap();
    assert_eq!((direct, callback), (false, false));
    assert!(cooling.is_empty() && harness.events.recorded().is_empty());
    assert_eq!(harness.service.counters.failures_for_minute("missing"), 0);
}

pub(super) async fn set_cools_a_known_deployment() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let direct = CooldownCall {
        deployment_id: "a",
        failure: &failure,
        exception: "probe failure",
        time_to_cooldown: None,
        requested_model_group: None,
    };
    assert!(
        harness
            .cooling(CooldownSource::Direct(&direct))
            .await
            .unwrap()
    );
    assert!(stored_record(&harness.shared, "a").is_some());
    assert_eq!(harness.events.recorded().len(), 1);
}

// Router.deployment_callback_on_failure with the cooldown time the case's header carries
async fn callback_cools(
    harness: &Harness,
    id: &str,
    failure: &FailureFacts,
    header: Option<Seconds>,
) -> bool {
    cooldown_result(harness, &attempt_with(id, failure, header, None)).await
}

// the ratio path, where one failure never cools a deployment that still has routable alternatives
async fn ratio_cools(fixture: Fixture, attempt: &CooldownAttempt<'_>) -> bool {
    let harness = harness(fixture, ratio_policy());
    cooldown_result(&harness, attempt).await
}

// _trigger_cooldown_for_failed_deployment with the server-stamped failed_deployment_id
async fn fallback_cools(harness: &Harness, failure: &FallbackFailure<'_>) -> bool {
    harness.fallback_trigger(failure).await
}

async fn cooldown_result(harness: &Harness, attempt: &CooldownAttempt<'_>) -> bool {
    harness
        .cooling(CooldownSource::Callback(attempt))
        .await
        .unwrap()
}

// the allowed_fails_policy entries a deployment declares
fn policy_json(allowed_fails: u32) -> Json {
    Json::Object(
        once((
            "RateLimitErrorAllowedFails".to_owned(),
            number(allowed_fails),
        ))
        .collect(),
    )
}
