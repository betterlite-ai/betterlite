use crate::otel::support::{
    PreCall, anchored, attribute, event, finished, harness, inference, owned_http, payload_start,
    pre_call, root_span_id, success, team_payload, with_id,
};

#[tokio::test]
async fn finish_renames_the_boundary_span_with_the_payload_model() {
    let harness = harness(owned_http(), anchored());
    pre_call(
        &harness.logger,
        PreCall {
            call_id: "call-a",
            model: "other-model",
            no_upstream: false,
            dynamic: None,
            auth: None,
        },
    );
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    let span = spans.first().expect("span");
    assert!(span.name.contains("gpt-4o"));
    assert!(!span.name.contains("other-model"));
    // born at the boundary, not at close: only the live span keeps the pre_call instant
    assert_ne!(span.start_time, payload_start());
}

#[tokio::test]
async fn finish_emits_a_deferred_span_seeded_with_identity_baggage() {
    let mut settings = owned_http();
    settings.baggage_promoted_keys = vec!["litellm.team.id".to_owned()];
    let harness = harness(settings, anchored());
    success(&harness.logger, event(with_id(team_payload(), "call-team"))).await;
    let spans = finished(&harness.spans);
    assert_eq!(spans.len(), 1);
    let span = spans.first().expect("span");
    assert_eq!(
        attribute(span, "litellm.team.id").as_deref(),
        Some("team-a")
    );
    assert_eq!(span.start_time, payload_start());
    // seeded from the request's anchored root, not from a detached context
    assert_eq!(span.parent_span_id, root_span_id());
}
