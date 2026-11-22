use std::sync::Arc;

use betterlite_models::decisions::Json;

use crate::cooldown_decisions::failure::{ErrorStatus, FailureClass, Seconds, TimeoutFacts};
use crate::cooldown_policy::service::{CooldownCall, CooldownSource};
use crate::routing_read_batch::{RequestReads, RoutingReadBatch};
use crate::routing_state_store::test_support::FakeRedis;

use super::support::{
    FailingObserver, Fixture, attempt, attempt_with, deployment, deployment_with, failure,
    fallback, float, harness, harness_sharing, harness_with, number, policy, rate_limit,
    ratio_policy, stored_record, text,
};

pub(super) async fn strict_threshold_and_one_event() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(1_u32));
    let lookup = harness.fixture.lookup();
    let attempt = attempt("a", &failure);
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    let record = stored_record(&harness.shared, "a").unwrap();
    assert_eq!(record.status_code, "429");
    assert_eq!(record.exception_received, "probe failure");
    assert_eq!(record.timestamp, 1000.0_f64);
    assert_eq!(record.cooldown_time, 5.0_f64);
    assert_eq!(harness.events.recorded().len(), 1);
}

pub(super) async fn disabled_cooldowns_count_the_failure_but_never_cool() {
    let mut settings = policy(0_u32);
    settings.disable_cooldowns = true;
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), settings);
    let lookup = harness.fixture.lookup();
    let attempt = attempt("a", &failure);
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt))
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
    let lookup = harness.fixture.lookup();
    let cooled = harness
        .service
        .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
        .await
        .unwrap();
    assert!(cooled);
    assert!(stored_record(&harness.shared, "a").is_some());
    assert_eq!(harness.events.recorded().len(), 1);
}

pub(super) async fn fallback_cools_the_server_stamped_deployment() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    assert!(
        harness
            .service
            .trigger_for_failed_deployment(&lookup, &fallback(Some("a"), &failure))
            .await
    );
    assert_eq!(harness.events.recorded().len(), 1);
    assert_eq!(harness.service.counters.failures_for_minute("a"), 1);
}

pub(super) async fn generic_404_and_caller_timeout_never_cool() {
    let generic = rate_limit(404);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    let mut generic_fallback = fallback(Some("a"), &generic);
    generic_fallback.original_generic_function = true;
    assert!(
        !harness
            .service
            .trigger_for_failed_deployment(&lookup, &generic_fallback)
            .await
    );
    // a background response cost poll's 404 says nothing about deployment health either
    let mut poll = rate_limit(404);
    poll.background_response_cost_poll = true;
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &poll)))
            .await
            .unwrap()
    );
    // the 404 guard covers only the generic calls a caller-supplied id can 404 on its own
    let direct = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let direct_lookup = direct.fixture.lookup();
    assert!(
        direct
            .service
            .trigger_for_failed_deployment(&direct_lookup, &fallback(Some("a"), &generic))
            .await
    );
    let timed_out = failure(FailureClass::Timeout, ErrorStatus::Code(408));
    let mut caller = timed_out.clone();
    caller.caller_timeout = Some(TimeoutFacts {
        client_side_timeout: true,
        timeout_seconds: Some(Seconds(1.0_f64)),
        elapsed_seconds: Some(Seconds(2.0_f64)),
    });
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &caller)))
            .await
            .unwrap()
    );
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
    let lookup = harness.fixture.lookup();
    let attempt = attempt_with("a", &failure, Some(Seconds(9.0_f64)), None);
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    assert_eq!(
        stored_record(&harness.shared, "a").unwrap().cooldown_time,
        3.0_f64
    );
    let params = deployment_with(
        "gpt",
        "b",
        vec![("cooldown_time", float(4.0_f64))],
        Vec::new(),
    );
    let params_harness = harness(Fixture::new(vec![params]), policy(0_u32));
    let params_lookup = params_harness.fixture.lookup();
    let params_attempt = attempt_with("b", &failure, Some(Seconds(9.0_f64)), None);
    assert!(
        params_harness
            .service
            .cool_deployment(&params_lookup, CooldownSource::Callback(&params_attempt))
            .await
            .unwrap()
    );
    assert_eq!(
        stored_record(&params_harness.shared, "b")
            .unwrap()
            .cooldown_time,
        4.0_f64
    );
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
    let lookup = harness.fixture.lookup();
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
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
    let lookup = harness.fixture.lookup();
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
    let events = harness.events.recorded();
    assert_eq!(events[0].api_base, "https://alias.example");
    assert_eq!(events[0].provider, "openai");
    let plain = harness_with(Fixture::new(vec![deployment("gpt", "b")]), policy(0_u32));
    let lookup = plain.fixture.lookup();
    assert!(
        plain
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("b", &failure)))
            .await
            .unwrap()
    );
    assert_eq!(
        plain.events.recorded()[0].api_base,
        "https://api.openai.com"
    );
}

pub(super) async fn alternatives_lift_the_single_deployment_net() {
    let failure = rate_limit(429);
    let fixture = Fixture::new(vec![deployment("gpt", "a"), deployment("other", "b")])
        .with_group("alt-group", &["gpt", "other"]);
    let harness = harness(fixture, ratio_policy());
    let lookup = harness.fixture.lookup();
    let grouped = attempt_with("a", &failure, None, Some("alt-group"));
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&grouped))
            .await
            .unwrap()
    );
    let alone = harness_with(Fixture::new(vec![deployment("gpt", "a")]), ratio_policy());
    let alone_lookup = alone.fixture.lookup();
    assert!(
        !alone
            .service
            .cool_deployment(
                &alone_lookup,
                CooldownSource::Callback(&attempt("a", &failure)),
            )
            .await
            .unwrap()
    );
    let team = vec![
        ("team_id", text("probe-team")),
        ("team_public_model_name", text("probe-public")),
    ];
    let team_fixture = Fixture::new(vec![
        deployment_with("gpt", "a", Vec::new(), team.clone()),
        deployment_with("other", "b", Vec::new(), team),
    ]);
    let team_harness = harness(team_fixture, ratio_policy());
    let team_lookup = team_harness.fixture.lookup();
    assert!(
        team_harness
            .service
            .cool_deployment(
                &team_lookup,
                CooldownSource::Callback(&attempt("a", &failure)),
            )
            .await
            .unwrap()
    );
    let blocked = vec![
        ("team_id", text("probe-team")),
        ("team_public_model_name", text("probe-public")),
        ("blocked", Json::Bool(true)),
    ];
    let blocked_fixture = Fixture::new(vec![
        deployment_with("gpt", "a", Vec::new(), blocked[..2].to_vec()),
        deployment_with("other", "b", Vec::new(), blocked),
    ]);
    let blocked_harness = harness(blocked_fixture, ratio_policy());
    let blocked_lookup = blocked_harness.fixture.lookup();
    assert!(
        !blocked_harness
            .service
            .cool_deployment(
                &blocked_lookup,
                CooldownSource::Callback(&attempt("a", &failure)),
            )
            .await
            .unwrap()
    );
}

pub(super) async fn provider_default_deployments_never_cool() {
    let failure = rate_limit(429);
    let fixture = Fixture::new(vec![deployment("gpt", "a")]).with_provider_default("a");
    let harness = harness(fixture, policy(0_u32));
    let lookup = harness.fixture.lookup();
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
    let ordinary = harness(Fixture::new(vec![deployment("gpt", "b")]), policy(0_u32));
    let ordinary_lookup = ordinary.fixture.lookup();
    assert!(
        ordinary
            .service
            .cool_deployment(
                &ordinary_lookup,
                CooldownSource::Callback(&attempt("b", &failure)),
            )
            .await
            .unwrap()
    );
}

pub(super) async fn debug_info_lists_only_the_cooling_deployment() {
    let failure = rate_limit(429);
    let harness = harness(
        Fixture::new(vec![deployment("gpt", "a"), deployment("gpt", "b")]),
        policy(0_u32),
    );
    let lookup = harness.fixture.lookup();
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
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
    let lookup = harness.fixture.lookup();
    assert!(
        !harness
            .service
            .trigger_for_failed_deployment(&lookup, &fallback(Some("a"), &failure))
            .await
    );
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
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
        vec![(
            "allowed_fails_policy",
            Json::Object(
                [("RateLimitErrorAllowedFails".to_owned(), number(0_u32))]
                    .into_iter()
                    .collect(),
            ),
        )],
    );
    let harness = harness(Fixture::new(vec![deployment]), policy(1_u32));
    let lookup = harness.fixture.lookup();
    let failure = failure(FailureClass::RateLimit, ErrorStatus::Text(String::new()));
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
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
        vec![(
            "allowed_fails_policy",
            Json::Object(
                [("RateLimitErrorAllowedFails".to_owned(), number(1_u32))]
                    .into_iter()
                    .collect(),
            ),
        )],
    );
    let counted_harness = harness(Fixture::new(vec![counted]), policy(5_u32));
    let counted_lookup = counted_harness.fixture.lookup();
    assert!(
        !counted_harness
            .service
            .cool_deployment(
                &counted_lookup,
                CooldownSource::Callback(&attempt("b", &failure)),
            )
            .await
            .unwrap()
    );
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
    let lookup = harness.fixture.lookup();
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt("a", &failure)))
            .await
            .unwrap()
    );
}

pub(super) async fn shared_fail_counts_answer_across_workers() {
    let server = FakeRedis::try_start().await.unwrap();
    let failure = rate_limit(429);
    let harness = harness_sharing(
        Fixture::new(vec![deployment("gpt", "a")]),
        policy(1_u32),
        &server,
    );
    let lookup = harness.fixture.lookup();
    let attempt = attempt("a", &failure);
    assert!(
        !harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Callback(&attempt))
            .await
            .unwrap()
    );
}

pub(super) async fn unknown_deployments_never_cool() {
    let failure = rate_limit(429);
    let harness = harness(Fixture::new(vec![deployment("gpt", "a")]), policy(0_u32));
    let lookup = harness.fixture.lookup();
    let missing = CooldownCall {
        deployment_id: "missing",
        failure: &failure,
        exception: "probe failure",
        time_to_cooldown: None,
        requested_model_group: None,
    };
    let direct = harness
        .service
        .cool_deployment(&lookup, CooldownSource::Direct(&missing))
        .await
        .unwrap();
    let callback = harness
        .service
        .cool_deployment(
            &lookup,
            CooldownSource::Callback(&attempt("missing", &failure)),
        )
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
    let lookup = harness.fixture.lookup();
    let direct = CooldownCall {
        deployment_id: "a",
        failure: &failure,
        exception: "probe failure",
        time_to_cooldown: None,
        requested_model_group: None,
    };
    assert!(
        harness
            .service
            .cool_deployment(&lookup, CooldownSource::Direct(&direct))
            .await
            .unwrap()
    );
    assert!(stored_record(&harness.shared, "a").is_some());
    assert_eq!(harness.events.recorded().len(), 1);
}
