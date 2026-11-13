#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use tokio::time::timeout;
use uuid::Uuid;

use super::support::{Call, FakeDatabase, RecordingIdentity, Step, capture_logs};
use crate::database_replica::pinned::WriterPinnedClient;
use crate::worker_heartbeat::{HostnameError, ProxyWorkerHeartbeat, count_live_proxy_workers};

#[rstest]
#[case::verbatim("worker-a")]
#[case::whitespace("  ")]
#[tokio::test]
async fn provided_worker_id_preserved(#[case] supplied: &str) {
    let database = FakeDatabase::default();
    let identity = RecordingIdentity::new("host-a");
    let service = ProxyWorkerHeartbeat::new(Some(supplied), &identity).unwrap();
    service.beat(&database).await;
    assert_eq!(identity.generated_calls(), 0);
    assert_eq!(
        database.calls(),
        vec![
            Call::Upsert {
                worker_id: supplied.to_owned(),
                hostname: "host-a".to_owned(),
            },
            Call::Prune {
                seconds: "3600".to_owned(),
            },
        ]
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn empty_id_generates_uuid_once(#[case] supplied: Option<&str>) {
    let database = FakeDatabase::default();
    let identity = RecordingIdentity::new("host-a");
    let service = ProxyWorkerHeartbeat::new(supplied, &identity).unwrap();
    assert_eq!(identity.generated_calls(), 1);
    service.beat(&database).await;
    service.beat(&database).await;
    assert_eq!(identity.generated_calls(), 1);
    let calls = database.calls();
    let Some(Call::Upsert { worker_id, .. }) = calls.first() else {
        panic!("the first call is the upsert");
    };
    let Some(Call::Upsert {
        worker_id: repeated,
        ..
    }) = calls.get(2)
    else {
        panic!("the third call is the second upsert");
    };
    assert_eq!(Uuid::parse_str(worker_id).unwrap().get_version_num(), 4);
    assert_eq!(worker_id, repeated);
}

#[tokio::test]
async fn hostname_captured_once() {
    let database = FakeDatabase::default();
    let identity = RecordingIdentity::new("host-a");
    let service = ProxyWorkerHeartbeat::new(Some("worker-a"), &identity).unwrap();
    service.beat(&database).await;
    service.deregister(&database).await;
    assert_eq!(identity.hostname_calls(), 1);
}

#[test]
fn hostname_error_propagates() {
    let identity = RecordingIdentity::failing_hostname();
    let result = ProxyWorkerHeartbeat::new(Some("worker-a"), &identity);
    assert!(matches!(result, Err(HostnameError::Provider)));
}

#[tokio::test]
async fn upsert_failure_skips_prune() {
    let capture = capture_logs();
    let database = FakeDatabase::default().failing(Step::Upsert);
    let identity = RecordingIdentity::new("host-a");
    let service = ProxyWorkerHeartbeat::new(Some("worker-a"), &identity).unwrap();
    service.beat(&database).await;
    assert_eq!(
        database.calls(),
        vec![Call::Upsert {
            worker_id: "worker-a".to_owned(),
            hostname: "host-a".to_owned(),
        }]
    );
    assert!(capture.logs.has(
        "DEBUG",
        "Proxy worker heartbeat write failed: upsert failed"
    ));
}

#[tokio::test]
async fn deregister_failure_debug() {
    let capture = capture_logs();
    let database = FakeDatabase::default().failing(Step::Deregister);
    let identity = RecordingIdentity::new("host-a");
    let service = ProxyWorkerHeartbeat::new(Some("worker-a"), &identity).unwrap();
    service.deregister(&database).await;
    assert_eq!(
        database.calls(),
        vec![Call::Deregister {
            worker_id: "worker-a".to_owned(),
        }]
    );
    assert!(capture.logs.has(
        "DEBUG",
        "Proxy worker heartbeat deregister failed: deregister failed"
    ));
}

#[tokio::test(start_paused = true)]
async fn cancellation_propagates() {
    let capture = capture_logs();
    let database = FakeDatabase::default().blocked();
    let identity = RecordingIdentity::new("host-a");
    let service = ProxyWorkerHeartbeat::new(Some("worker-a"), &identity).unwrap();
    let outcome = timeout(Duration::from_secs(1), service.beat(&database)).await;
    assert!(outcome.is_err());
    assert_eq!(
        database.calls(),
        vec![Call::Upsert {
            worker_id: "worker-a".to_owned(),
            hostname: "host-a".to_owned(),
        }]
    );
    assert!(!capture.logs.contents().contains("write failed"));
}

#[tokio::test]
async fn count_zero_is_known() {
    let database = FakeDatabase::default().with_counts(vec![Some(0)]);
    let count = count_live_proxy_workers(WriterPinnedClient::Writer(&database)).await;
    assert_eq!(count, Some(0));
    assert_eq!(
        database.calls(),
        vec![Call::Count {
            seconds: "180".to_owned(),
        }]
    );
}

#[rstest]
#[case::empty(vec![], "the result has no live worker count row")]
#[case::null_row(vec![None], "a live worker count row is not an integer")]
#[case::invalid_tail(
    vec![Some(1), None],
    "a live worker count row is not an integer"
)]
#[tokio::test]
async fn count_empty_none_malformed_unknown(
    #[case] rows: Vec<Option<i32>>,
    #[case] expected: &str,
) {
    let capture = capture_logs();
    let database = FakeDatabase::default().with_counts(rows);
    let count = count_live_proxy_workers(WriterPinnedClient::Writer(&database)).await;
    assert_eq!(count, None);
    let message = format!("Live proxy worker count unavailable: {expected}");
    assert!(capture.logs.has("DEBUG", &message));
}

#[rstest]
#[case::first_wins(vec![Some(3), Some(9)], Some(3))]
#[case::later_row_validated(vec![Some(3), None], None)]
#[tokio::test]
async fn count_validates_all_rows_uses_first(
    #[case] rows: Vec<Option<i32>>,
    #[case] expected: Option<i32>,
) {
    let database = FakeDatabase::default().with_counts(rows);
    let count = count_live_proxy_workers(WriterPinnedClient::Writer(&database)).await;
    assert_eq!(count, expected);
}

#[tokio::test]
async fn count_failure_debug_unknown() {
    let capture = capture_logs();
    let database = FakeDatabase::default().failing(Step::Count);
    let count = count_live_proxy_workers(WriterPinnedClient::Writer(&database)).await;
    assert_eq!(count, None);
    assert!(
        capture
            .logs
            .has("DEBUG", "Live proxy worker count unavailable: count failed")
    );
}
