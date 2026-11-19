use rstest::rstest;

use super::*;

#[tokio::test]
async fn empty_payload_makes_no_event_and_no_timing_change() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    assert!(matches!(rig.process("").await, Ok(None)));
    assert!(!rig.state.finished);
    assert_eq!(completion_start(&rig.logging), None);
    assert!(rig.ports.success.calls.entries().is_empty());
}

#[rstest]
#[case::done("[DONE]", true)]
#[case::spaced_done("[DONE] ", false)]
#[tokio::test]
async fn exact_done_marker_finishes_without_an_event(
    #[case] payload: &str,
    #[case] finished: bool,
) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    assert!(matches!(rig.process(payload).await, Ok(None)));
    assert_eq!(rig.state.finished, finished);
    let expected_start = if finished {
        None
    } else {
        Some(timestamp(CLOCK))
    };
    assert_eq!(completion_start(&rig.logging), expected_start);
    assert!(rig.ports.failure.calls.entries().is_empty());
}

#[rstest]
#[case::not_json("{not json")]
#[case::empty_list("[]")]
#[case::null("null")]
#[case::number("1")]
#[case::string("\"text\"")]
#[case::data_prefix(r#"data:{"type":"response.created"}"#)]
#[tokio::test]
async fn malformed_and_non_object_payloads_are_dropped(#[case] payload: &str) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    assert!(
        matches!(rig.process(payload).await, Ok(None)),
        "payload {payload} must drop"
    );
    assert!(!rig.state.finished);
    assert_eq!(completion_start(&rig.logging), Some(timestamp(CLOCK)));
    await_pending_logging(&mut rig.callback).await;
    assert!(rig.ports.failure.calls.entries().is_empty());
}

#[tokio::test]
async fn completion_start_time_is_set_once_before_parsing() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.clock = Arc::new(TickingClock {
        calls: Mutex::new(0),
        start: timestamp(START),
    });
    let started = timestamp(START) + chrono::TimeDelta::seconds(1);
    rig.process("{not json")
        .await
        .expect("a malformed payload is skipped");
    assert_eq!(completion_start(&rig.logging), Some(started));
    rig.process(TEXT_DELTA)
        .await
        .expect("the delta is handed on");
    assert_eq!(completion_start(&rig.logging), Some(started));
}

#[tokio::test]
async fn missing_provider_config_fails_once_with_the_litellm_message() {
    let mut rig = Rig::<TransformMode>::new(None);
    let Err(error) = rig.process(CREATED).await else {
        panic!("a payload without a provider config must fail");
    };
    assert!(matches!(&error, ProcessEventError::MissingProviderConfig));
    assert_eq!(
        error.to_string(),
        "responses_api_provider_config is required to process live streaming chunks"
    );
    let second = rig.process(CREATED).await.unwrap_err();
    assert!(matches!(second, ProcessEventError::MissingProviderConfig));
    await_pending_logging(&mut rig.callback).await;
    assert!(rig.state.failure_handled);
    assert_eq!(rig.ports.failure.calls.entries(), vec!["async", "sync"]);
    let deliveries = rig.ports.failure.deliveries();
    assert_eq!(deliveries.len(), 1);
    assert!(deliveries[0].contains("responses_api_provider_config is required"));
    assert!(rig.ports.success.calls.entries().is_empty());
}

#[tokio::test]
async fn transform_errors_propagate_and_notify_failure_once() {
    let mut rig = Rig::new(Some(TransformMode::Rejecting));
    let Err(error) = rig.process(TEXT_DELTA).await else {
        panic!("a rejected transform must surface");
    };
    assert!(matches!(&error, ProcessEventError::Transform(_)));
    assert_eq!(error.to_string(), "the provider transform failed");
    await_pending_logging(&mut rig.callback).await;
    assert_eq!(rig.ports.failure.calls.entries(), vec!["async", "sync"]);
    assert_eq!(rig.ports.failure.deliveries().len(), 1);
}

#[tokio::test]
async fn provider_transform_receives_model_chunk_and_details() {
    let (transform, seen) = TransformMode::recording();
    let mut rig = Rig::new(Some(transform));
    rig.process(TEXT_DELTA)
        .await
        .expect("the delta is processed");
    let entries = seen.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "gpt-4o");
    let parsed: Json = serde_json::from_str(TEXT_DELTA).expect("the fixture parses");
    assert_eq!(entries[0].1, parsed);
    assert_eq!(
        rig.details.get("transform_type"),
        Some(&Json::Text("response.output_text.delta".to_owned()))
    );
    assert_eq!(rig.state.generated_content, "hi");
}

#[tokio::test]
async fn quota_protocol_is_exposed_without_a_per_event_call() {
    let counter = Arc::new(QuotaCounter {
        calls: Mutex::new(0),
    });
    let port: Arc<dyn ProjectQuotaCallback> = Arc::<QuotaCounter>::clone(&counter);
    port.enforce_frame_quota(None, Some("gpt-4o"), 1, 2).await;
    assert_eq!(counter.calls(), 1);
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.process(COMPLETED)
        .await
        .expect("the terminal is processed");
    assert_eq!(counter.calls(), 1);
}

#[tokio::test]
async fn payloads_over_the_event_limit_are_dropped_like_malformed_json() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    let padding = "x".repeat(rig.limits.max_event_bytes);
    let payload = format!(
        r#"{{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi","pad":"{padding}"}}"#
    );
    assert!(matches!(rig.process(&payload).await, Ok(None)));
    assert_eq!(rig.state.generated_content, "");
    assert!(rig.state.completed_response.is_none());
    assert_eq!(completion_start(&rig.logging), Some(timestamp(CLOCK)));
    assert!(rig.ports.failure.calls.entries().is_empty());
}

#[tokio::test]
async fn payloads_at_the_event_limit_still_reach_the_provider_transform() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    let prefix = r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi","pad":""#;
    let padding = "x".repeat(rig.limits.max_event_bytes - prefix.len() - 2);
    let payload = format!("{prefix}{padding}\"}}");
    assert_eq!(payload.len(), rig.limits.max_event_bytes);
    rig.process(&payload)
        .await
        .expect("the payload at the limit is processed")
        .expect("the payload at the limit is handed on");
    assert_eq!(rig.state.generated_content, "hi");
    assert_eq!(completion_start(&rig.logging), Some(timestamp(CLOCK)));
}
