use std::time::SystemTime;

use betterlite_logging::logging_pre_post::hooks::PreCallEvent;
use betterlite_logging::otel::logger::pre_call::log_pre_api_call;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::otel::support::{
    PreCall, anchored, creds, empty_trace, event, event_with, finished, harness, inference,
    json_object, owned_http, payload_start, pre_call, success, with_id,
};

#[tokio::test]
async fn pre_call_opens_the_span_and_holds_the_route_at_the_boundary() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S")),
            auth: None,
        },
    );
    assert_eq!(harness.built.builds(), 1);
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    // the live span closes without routing again
    assert_eq!(harness.built.builds(), 1);
    let spans = finished(&harness.built.sink(0));
    assert_eq!(spans.len(), 1);
    assert_ne!(spans.first().expect("span").start_time, payload_start());
}

#[tokio::test]
async fn pre_call_defers_the_span_and_routes_at_close_without_a_parent() {
    let harness = harness(owned_http(), empty_trace());
    let before = SystemTime::now();
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S")),
            auth: None,
        },
    );
    assert_eq!(harness.built.builds(), 0);
    success(
        &harness.logger,
        event_with(with_id(inference(), "call-a"), creds("S"), IndexMap::new()),
    )
    .await;
    assert_eq!(harness.built.builds(), 1);
    let spans = finished(&harness.built.sink(0));
    assert_eq!(spans.len(), 1);
    let span = spans.first().expect("span");
    // the close opens at the carrier's pre_call instant, not at the payload's 2024 start
    assert!(span.start_time >= before);
    assert_ne!(span.start_time, payload_start());
}

#[tokio::test]
async fn pre_call_skips_a_synthetic_no_upstream_call() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: true,
            dynamic: Some(creds("S")),
            auth: None,
        },
    );
    assert_eq!(harness.built.builds(), 0);
    // no carrier was stored, so a real boundary for the same id still opens one
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S")),
            auth: None,
        },
    );
    assert_eq!(harness.built.builds(), 1);
}

#[tokio::test]
async fn pre_call_keeps_the_first_carrier_for_a_repeated_call_id() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S1")),
            auth: None,
        },
    );
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "gpt-4o",
            no_upstream: false,
            dynamic: Some(creds("S2")),
            auth: None,
        },
    );
    assert_eq!(harness.built.builds(), 1);
    let specs = harness.built.specs();
    assert_eq!(specs.len(), 1);
    let headers = specs
        .first()
        .and_then(|spec| spec.headers.clone())
        .unwrap_or_default();
    assert!(headers.contains("arize-space-id=S1"));
    assert!(!headers.contains("S2"));
}

#[tokio::test]
async fn pre_call_ignores_a_boundary_without_a_call_id() {
    let harness = harness(owned_http(), anchored());
    let details = json_object(vec![
        ("model", Json::Text("gpt-4o".to_owned())),
        ("call_type", Json::Text("completion".to_owned())),
        (
            "standard_callback_dynamic_params",
            json_object(vec![
                ("arize_space_id", Json::Text("S".to_owned())),
                ("arize_api_key", Json::Text("K".to_owned())),
            ]),
        ),
    ]);
    let messages = Json::List(Vec::new());
    log_pre_api_call(
        &harness.logger,
        &PreCallEvent {
            model: "gpt-4o",
            messages: &messages,
            details: &details,
        },
    )
    .expect("pre api call");
    assert_eq!(harness.built.builds(), 0);
}
