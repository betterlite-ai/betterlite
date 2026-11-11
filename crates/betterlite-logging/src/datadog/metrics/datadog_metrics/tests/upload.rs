use betterlite_models::decisions::Json;

use crate::datadog::metrics::flush::flush_queue;
use crate::datadog::metrics::measurements::RequestTiming;
use crate::datadog::metrics::series::DatadogMetricsPayload;

use super::support::{self, MetricsInit, Outcome};

const API_KEY: &str = "secret-key";

fn environment(pairs: &[(&str, &str)]) -> support::Environment {
    support::environment(pairs)
}

#[tokio::test]
async fn upload_gzips_the_payload_and_sets_the_api_key_headers() {
    let environment = environment(&[
        ("DD_API_KEY", API_KEY),
        ("DD_APP_KEY", "app-key"),
        ("DD_SITE", "datadoghq.eu"),
    ]);
    let (logger, transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
    );
    let payload = support::payload();
    let expected = support::metrics_for(
        &logger,
        &payload,
        "200",
        RequestTiming {
            start_time: Some(payload.start_time),
            end_time: Some(payload.end_time),
            api_call_start_time: None,
        },
    );
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let report = flush_queue(&logger).await;
    assert_eq!(report.batch, 2);
    assert_eq!(report.delivered, 2);
    assert_eq!(report.requeued, 0);
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.url, "https://api.datadoghq.eu/api/v2/series");
    assert_eq!(call.header("content-encoding"), Some("gzip"));
    assert_eq!(call.header("content-type"), Some("application/json"));
    assert_eq!(call.header("dd-api-key"), Some(API_KEY));
    assert_eq!(call.header("dd-application-key"), Some("app-key"));
    let body = serde_json::to_vec(&DatadogMetricsPayload { series: expected })
        .expect("the expected payload serializes");
    assert_eq!(call.body, body);
}

#[tokio::test]
async fn upload_defaults_the_site_to_datadoghq_com() {
    let environment = environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let _report = flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls[0].url, "https://api.datadoghq.com/api/v2/series");
    assert_eq!(calls[0].header("dd-application-key"), None);
}

#[tokio::test]
async fn upload_treats_an_empty_site_as_unset() {
    let environment = environment(&[("DD_API_KEY", API_KEY), ("DD_SITE", "")]);
    let (logger, transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let _report = flush_queue(&logger).await;
    assert_eq!(
        transport.calls()[0].url,
        "https://api.datadoghq.com/api/v2/series"
    );
}

#[tokio::test]
async fn upload_skips_without_an_api_key_and_clears_the_queue() {
    let (logger, transport) = support::build(
        &support::environment(&[]),
        MetricsInit::default(),
        Vec::new(),
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let report = flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 0);
    assert_eq!(report.delivered, 2);
    assert!(logger.queue.pending().is_empty());
}

#[tokio::test]
async fn upload_treats_an_empty_api_key_as_absent() {
    let environment = environment(&[("DD_API_KEY", "")]);
    let (logger, transport) = support::build(&environment, MetricsInit::default(), Vec::new());
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let _report = flush_queue(&logger).await;
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn upload_keeps_the_queue_when_the_api_answers_an_error() {
    let environment = environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(500), Outcome::Reply(202)],
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let first = flush_queue(&logger).await;
    assert_eq!(first.delivered, 0);
    assert_eq!(first.requeued, 2);
    assert_eq!(logger.queue.pending().len(), 2);
    let second = flush_queue(&logger).await;
    assert_eq!(second.delivered, 2);
    assert!(logger.queue.pending().is_empty());
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn upload_keeps_the_queue_when_the_transport_fails() {
    let environment = environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, _transport) =
        support::build(&environment, MetricsInit::default(), vec![Outcome::Network]);
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let report = flush_queue(&logger).await;
    assert_eq!(report.delivered, 0);
    assert_eq!(report.requeued, 2);
    assert_eq!(logger.queue.pending().len(), 2);
}

#[tokio::test]
async fn upload_keeps_the_queue_when_the_api_answers_a_redirect() {
    let environment = environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, _transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(302)],
    );
    let payload = support::payload();
    let source = support::source(&logger, &payload, "200");
    logger.record(&source).await;
    let report = flush_queue(&logger).await;
    assert_eq!(report.delivered, 0);
    assert_eq!(report.requeued, 2);
    assert_eq!(logger.queue.pending().len(), 2);
}

#[tokio::test]
async fn upload_sends_every_queued_series_in_one_request() {
    let environment = environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
    );
    let payload = support::payload();
    let first = support::source(&logger, &payload, "200");
    logger.record(&first).await;
    let second = support::source(&logger, &payload, "200");
    logger.record(&second).await;
    let report = flush_queue(&logger).await;
    assert_eq!(report.batch, 4);
    assert_eq!(transport.call_count(), 1);
    let call = &transport.calls()[0];
    let Json::Object(fields) = call.json() else {
        panic!("the payload is a JSON object");
    };
    let Some(Json::List(series)) = fields.get("series") else {
        panic!("the payload carries a series list");
    };
    assert_eq!(series.len(), 4);
}
