use rstest::rstest;

use super::support::{Fixture, Reply, capture_logs, record_at};
use crate::newrelic_metrics::flush::async_send_batch;
use crate::newrelic_metrics::queue::{push_record, queue_length};

#[rstest]
#[case::accepted(Reply::Status(202), true, 0)]
#[case::no_content(Reply::Status(204), true, 0)]
#[case::request_timeout(Reply::Status(408), false, 1)]
#[case::rate_limited(Reply::Status(429), false, 1)]
#[case::server_error(Reply::Status(500), false, 1)]
#[case::unavailable(Reply::Status(503), false, 1)]
#[case::forbidden(Reply::Status(403), true, 0)]
#[case::bad_request(Reply::Status(400), true, 0)]
#[case::not_found(Reply::Status(404), true, 0)]
#[case::unprocessable(Reply::Status(422), true, 0)]
#[tokio::test]
async fn response_classification_settles_each_status(
    #[case] reply: Reply,
    #[case] delivered: bool,
    #[case] depth: usize,
) {
    let fixture = Fixture::new_with_replies(&[reply]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert_eq!(async_send_batch(&logger).await, delivered);
    assert_eq!(queue_length(&logger), depth);
}

#[tokio::test]
async fn response_classification_requeues_a_network_error() {
    let fixture = Fixture::new_with_replies(&[Reply::Network]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(!async_send_batch(&logger).await);
    assert_eq!(queue_length(&logger), 1);
}

#[tokio::test]
async fn response_classification_names_the_403_credential_failure() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new_with_replies(&[Reply::Status(403)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let text = logs.text();
    assert!(text.contains("403"));
    assert!(text.contains("permanent credential failure"));
}

#[tokio::test]
async fn response_classification_names_the_dropped_client_error() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new_with_replies(&[Reply::Status(400)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let text = logs.text();
    assert!(text.contains("client error from Metric API, dropping records"));
}

#[tokio::test]
async fn response_classification_names_the_network_retry() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new_with_replies(&[Reply::Network]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(!async_send_batch(&logger).await);
    assert!(
        logs.text()
            .contains("network error sending records, will retry")
    );
}

#[tokio::test]
async fn response_classification_names_the_status_retry() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::new_with_replies(&[Reply::Status(500)]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(!async_send_batch(&logger).await);
    assert!(
        logs.text()
            .contains("retryable status from Metric API, will retry records")
    );
}
