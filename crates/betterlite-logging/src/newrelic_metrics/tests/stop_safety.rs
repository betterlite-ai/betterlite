use std::sync::atomic::Ordering;

use tokio::runtime::Builder;

use super::support::{Fixture, Reply, record_at};
use crate::newrelic_metrics::queue::{push_record, queue_length};

#[tokio::test]
async fn stop_safety_schedules_a_final_drain() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(202)]);
    let logger = fixture.build();
    fixture.end_periodic_flush().await;
    push_record(&logger, record_at(0));
    assert!(logger.stop());
    fixture.wait_for_tasks().await;
    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(queue_length(&logger), 0);
}

#[tokio::test]
async fn stop_safety_marks_the_logger_stopped() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    assert!(logger.stop());
    assert!(logger.stopped.load(Ordering::SeqCst));
    fixture.wait_for_tasks().await;
}

#[tokio::test]
async fn stop_safety_schedules_one_drain_per_call() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    fixture.end_periodic_flush().await;
    assert_eq!(fixture.tasks.len(), 0);
    assert!(logger.stop());
    assert_eq!(fixture.tasks.len(), 1);
    assert!(logger.stop());
    assert_eq!(fixture.tasks.len(), 2);
    fixture.wait_for_tasks().await;
}

#[tokio::test]
async fn stop_safety_drains_a_record_after_a_failing_first_pass() {
    let fixture = Fixture::new_with_replies(&[Reply::Status(500), Reply::Status(202)]);
    let logger = fixture.build();
    fixture.end_periodic_flush().await;
    push_record(&logger, record_at(0));
    assert!(logger.stop());
    fixture.wait_for_tasks().await;
    assert_eq!(fixture.transport.requests().len(), 2);
    assert_eq!(queue_length(&logger), 0);
}

#[test]
fn stop_safety_outside_a_runtime_returns_false() {
    let runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a current thread runtime");
    let fixture = Fixture::new();
    let logger = runtime.block_on(async { fixture.build() });
    drop(runtime);
    assert!(!logger.stop());
    assert_eq!(queue_length(&logger), 0);
}
