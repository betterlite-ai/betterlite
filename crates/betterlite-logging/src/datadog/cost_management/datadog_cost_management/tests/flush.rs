use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::advance;

use super::support::{self, Fixture};

// a deadline at exactly the interval can sit one millisecond short, so each advance passes it
const BEFORE_INTERVAL: Duration = Duration::from_millis(4_000);
const PAST_INTERVAL: Duration = Duration::from_millis(5_001);

#[tokio::test]
async fn flush_a_direct_threshold_send_uploads_the_queue() {
    let mut init = support::init(&[]);
    init.batch_size = Some(1);
    let fixture = Fixture::new(&[202]).with_init(init);
    let logger = fixture.build();
    assert!(logger.log_success(support::default_payload().build()).await);
    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 1);
    let request = requests.first().expect("the upload request");
    assert_eq!(
        request.url,
        "https://api.test.datadoghq.com/api/v2/cost/custom_costs"
    );
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        request.headers.get("dd-api-key").map(String::as_str),
        Some("api-key")
    );
    assert_eq!(
        request
            .headers
            .get("dd-application-key")
            .map(String::as_str),
        Some("app-key")
    );
    assert_eq!(logger.queue_length(), 0);
}

#[tokio::test]
async fn flush_without_credentials_skips_the_upload_and_drains_the_queue() {
    let mut fixture = Fixture::new(&[]);
    fixture.environment.remove("DD_APP_KEY");
    let logger = fixture.build();
    assert!(logger.log_success(support::default_payload().build()).await);
    let report = logger.flush_queue().await;
    assert_eq!(report.batch, 1);
    assert!(fixture.transport.requests().is_empty());
    assert_eq!(logger.queue_length(), 0);
}

#[tokio::test(start_paused = true)]
async fn flush_the_periodic_timer_uploads_the_queue() {
    let fixture = Fixture::new(&[202]).with_tasks();
    let logger = fixture.build();
    yield_now().await;
    assert!(logger.log_success(support::default_payload().build()).await);
    assert!(fixture.transport.requests().is_empty());
    advance(BEFORE_INTERVAL).await;
    yield_now().await;
    assert!(fixture.transport.requests().is_empty());
    advance(PAST_INTERVAL).await;
    yield_now().await;
    yield_now().await;
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(logger.queue_length(), 0);
}

#[tokio::test(start_paused = true)]
async fn flush_a_failed_periodic_send_keeps_the_batch() {
    let fixture = Fixture::new(&[500]).with_tasks();
    let logger = fixture.build();
    yield_now().await;
    assert!(logger.log_success(support::default_payload().build()).await);
    advance(PAST_INTERVAL).await;
    yield_now().await;
    yield_now().await;
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(logger.queue_length(), 1);
}

#[tokio::test(start_paused = true)]
async fn flush_stops_when_the_shutdown_token_is_cancelled() {
    let fixture = Fixture::new(&[202]).with_tasks();
    let logger = fixture.build();
    yield_now().await;
    fixture.shutdown.cancel();
    advance(PAST_INTERVAL).await;
    yield_now().await;
    yield_now().await;
    assert!(logger.log_success(support::default_payload().build()).await);
    advance(PAST_INTERVAL).await;
    yield_now().await;
    yield_now().await;
    assert!(fixture.transport.requests().is_empty());
    assert_eq!(logger.queue_length(), 1);
}
