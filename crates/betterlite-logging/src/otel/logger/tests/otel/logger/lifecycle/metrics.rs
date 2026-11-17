use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_models::decisions::Json;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::otel::support::{
    Captured, PreCall, anchored, event, event_with, failure, harness_with, inference,
    inference_failure, metric_points, no_upstream, number, owned_http, pre_call, recorder,
    stream_event, success, token_usage, with_id, with_response, zero_start,
};

#[tokio::test]
async fn metrics_record_a_successful_call() {
    let (recorder, provider, exporter) = recorder(None);
    let harness = harness_with(owned_http(), anchored(), Some(recorder));
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    // the duration, the cost and the api latency, which runs from the payload's start to its end
    assert_eq!(metric_points(&provider, &exporter), 3);
}

#[tokio::test]
async fn metrics_measure_the_first_chunk_from_the_carrier_start() {
    let (recorder, provider, exporter) = recorder(None);
    let harness = harness_with(owned_http(), anchored(), Some(recorder));
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
    success(
        &harness.logger,
        stream_event(with_id(inference(), "call-a")),
    )
    .await;
    // the first chunk runs from the boundary, which postdates the payload end, so the negative api latency is skipped: the duration, the cost and the chunk
    assert_eq!(metric_points(&provider, &exporter), 3);
}

#[tokio::test]
async fn metrics_skip_the_first_chunk_without_a_start_stamp() {
    let (recorder, provider, exporter) = recorder(None);
    let harness = harness_with(owned_http(), anchored(), Some(recorder));
    success(
        &harness.logger,
        stream_event(with_id(zero_start(inference()), "call-a")),
    )
    .await;
    // a zero start stamp leaves only the duration and the cost, even for a stream
    assert_eq!(metric_points(&provider, &exporter), 2);
}

#[tokio::test]
async fn metrics_stay_silent_about_a_non_filter_error() {
    let (recorder, _, _) = recorder(None);
    let harness = harness_with(owned_http(), anchored(), Some(recorder));
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let _guard = set_default(subscriber);
    success(
        &harness.logger,
        event(with_id(
            with_response(
                inference(),
                token_usage(Json::Text("many".to_owned()), number(3)),
            ),
            "call-a",
        )),
    )
    .await;
    // only an otel.attributes filter error is an operator fix worth an ERROR record
    assert!(captured.messages().is_empty());
}

#[tokio::test]
async fn metrics_record_a_failed_call() {
    let (recorder, provider, exporter) = recorder(None);
    let harness = harness_with(owned_http(), anchored(), Some(recorder));
    failure(
        &harness.logger,
        event(with_id(inference_failure(), "call-b")),
    )
    .await;
    assert_ne!(metric_points(&provider, &exporter), 0);
}

#[tokio::test]
async fn metrics_skip_a_synthetic_no_upstream_failure() {
    let (recorder, provider, exporter) = recorder(None);
    let harness = harness_with(owned_http(), anchored(), Some(recorder));
    failure(
        &harness.logger,
        event_with(
            with_id(inference_failure(), "call-c"),
            Json::Null,
            no_upstream(),
        ),
    )
    .await;
    assert_eq!(metric_points(&provider, &exporter), 0);
}

#[tokio::test]
async fn metrics_log_a_filter_error_once_and_then_stay_silent() {
    let (recorder, _, _) = recorder(Some(OtelModelValue::String("not a mapping".to_owned())));
    let harness = harness_with(owned_http(), anchored(), Some(recorder));
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let _guard = set_default(subscriber);
    success(&harness.logger, event(with_id(inference(), "call-a"))).await;
    // the first record surfaces the filter error
    assert_eq!(captured.messages().len(), 1);
    success(&harness.logger, event(with_id(inference(), "call-b"))).await;
    let messages = captured.messages();
    assert_eq!(messages.len(), 1);
    assert!(
        messages
            .first()
            .expect("message")
            .contains("invalid otel.attributes metric filter")
    );
}
