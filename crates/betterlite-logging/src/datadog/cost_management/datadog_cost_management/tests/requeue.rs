use betterlite_models::decisions::Json;

use super::support::{self, Fixture};

#[tokio::test]
async fn requeue_prepends_the_failed_batch_in_order() {
    let fixture = Fixture::new(&[500]);
    let logger = fixture.build();
    let first = support::default_payload().build();
    let second = support::default_payload()
        .set("/id", support::text("chatcmpl-2"))
        .build();
    assert!(logger.log_success(first).await);
    assert!(logger.log_success(second).await);
    let report = logger.flush_queue().await;
    assert_eq!(report.batch, 2);
    assert_eq!(report.requeued, 2);
    assert_eq!(logger.queue_length(), 2);
    let pending = logger.queue.pending();
    assert_eq!(
        pending.first().map(|payload| payload.id.as_str()),
        Some("chatcmpl-1")
    );
    assert_eq!(
        pending.last().map(|payload| payload.id.as_str()),
        Some("chatcmpl-2")
    );
}

#[tokio::test]
async fn requeue_keeps_the_direct_threshold_batch() {
    let mut init = support::init(&[]);
    init.batch_size = Some(1);
    let fixture = Fixture::new(&[500]).with_init(init);
    let logger = fixture.build();
    assert!(logger.log_success(support::default_payload().build()).await);
    assert_eq!(logger.queue_length(), 1);
    assert_eq!(fixture.transport.requests().len(), 1);
}

#[tokio::test]
async fn requeue_keeps_a_batch_the_service_answered_without_success() {
    let mut init = support::init(&[]);
    init.batch_size = Some(1);
    let fixture = Fixture::new(&[307]).with_init(init);
    let logger = fixture.build();
    assert!(logger.log_success(support::default_payload().build()).await);
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(logger.queue_length(), 1);
}

#[tokio::test]
async fn requeue_retries_the_original_order() {
    let fixture = Fixture::new(&[500, 202]);
    let logger = fixture.build();
    let first = support::default_payload().set("/model", support::text("first-model"));
    let second = support::default_payload().set("/model", support::text("second-model"));
    assert!(logger.log_success(first.build()).await);
    assert!(logger.log_success(second.build()).await);
    logger.flush_queue().await;
    logger.flush_queue().await;
    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 2);
    let retried = requests.last().expect("the retry request");
    let Json::List(entries) = &retried.body else {
        panic!("the upload body is a JSON array");
    };
    let descriptions: Vec<Option<&str>> = entries
        .iter()
        .map(|entry| {
            support::object_field(entry, "ChargeDescription").and_then(support::text_value)
        })
        .collect();
    assert_eq!(
        descriptions,
        [
            Some("LLM Usage for first-model"),
            Some("LLM Usage for second-model")
        ]
    );
    assert_eq!(logger.queue_length(), 0);
}
