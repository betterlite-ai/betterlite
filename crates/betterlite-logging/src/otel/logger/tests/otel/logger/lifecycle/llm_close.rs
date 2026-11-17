use betterlite_logging::otel::logger::call_context::SpanTimes;
use betterlite_logging::otel::logger::finish::close_llm_call;
use betterlite_logging::otel::plumbing::routing::cache::MAX_CACHED_PROVIDERS;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::SpanId;

use crate::otel::support::{
    PreCall, anchored, attribute, close_credentialed_tenants, creds, empty_trace, event,
    event_with, failure, finished, harness, inference, inference_failure, no_upstream, owned_http,
    payload_end, payload_start, payloadless_call, pre_call, root_trace_id, success, team_payload,
    with_id, zero_start,
};

#[tokio::test]
async fn llm_close_dedups_the_second_callback_for_one_call_id() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: None,
            auth: None,
        },
    );
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    failure(
        &harness.logger,
        event(with_id(inference_failure(), "call-a")),
    )
    .await;
    assert_eq!(finished(&harness.spans).len(), 1);
}

#[tokio::test]
async fn llm_close_emits_deferred_when_a_payload_proves_the_call_ran() {
    let harness = harness(owned_http(), anchored());
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans.first().expect("span").start_time, payload_start());
}

#[tokio::test]
async fn deferred_close_releases_the_routed_providers_lease() {
    let harness = harness(owned_http(), empty_trace());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S0")),
            auth: None,
        },
    );
    // no parent is recordable, so nothing routes before the close itself does
    assert_eq!(harness.built.builds(), 0);
    success(
        &harness.logger,
        event_with(with_id(inference(), "call-a"), creds("S0"), IndexMap::new()),
    )
    .await;
    assert_eq!(harness.built.builds(), 1);
    // every later close routes its own tenant and releases it, so S0's provider drains to zero
    close_credentialed_tenants(&harness, MAX_CACHED_PROVIDERS).await;
    // the deferred close released S0's lease, so the overflow eviction shuts its provider down at once
    assert!(harness.built.sink(0).is_shutdown_called());
}

#[tokio::test]
async fn llm_close_skips_a_synthetic_no_upstream_call_without_a_carrier() {
    let harness = harness(owned_http(), anchored());
    failure(
        &harness.logger,
        event_with(
            with_id(inference_failure(), "call-a"),
            Json::Null,
            no_upstream(),
        ),
    )
    .await;
    assert!(finished(&harness.spans).is_empty());
}

#[tokio::test]
async fn llm_close_skips_a_call_that_never_reached_the_provider() {
    let harness = harness(owned_http(), anchored());
    success(
        &harness.logger,
        event(with_id(zero_start(inference()), "call-a")),
    )
    .await;
    assert!(finished(&harness.spans).is_empty());
}

#[test]
fn llm_close_ends_the_boundary_span_when_no_payload_arrived() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: None,
            auth: None,
        },
    );
    let times = SpanTimes {
        start: Some(payload_start()),
        end: Some(payload_end()),
    };
    let closed =
        close_llm_call(&harness.logger, &payloadless_call("call-a"), times).expect("close");
    assert!(closed.is_some());
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    // nothing but the callback closes this span, so it ends at the callback's time, not at drop
    assert_eq!(spans.first().expect("span").end_time, payload_end());
}

#[tokio::test]
async fn llm_close_keeps_identity_baggage_on_a_detached_route() {
    let mut settings = owned_http();
    settings.baggage_promoted_keys = vec!["litellm.team.id".to_owned()];
    let harness = harness(settings, anchored());
    success(
        &harness.logger,
        event_with(
            with_id(team_payload(), "call-team"),
            creds("S"),
            IndexMap::new(),
        ),
    )
    .await;
    let spans = finished(&harness.built.sink(0));
    assert_eq!(spans.len(), 1);
    let span = spans.first().expect("span");
    assert_eq!(
        attribute(span, "litellm.team.id").as_deref(),
        Some("team-a")
    );
    // the routed span roots its own trace and links back to the request's root
    assert_eq!(span.parent_span_id, SpanId::INVALID);
    assert!(
        span.links
            .iter()
            .any(|link| link.span_context.trace_id() == root_trace_id())
    );
}
