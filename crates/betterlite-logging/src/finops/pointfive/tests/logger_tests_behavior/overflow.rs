use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_config::sections::pointfive_params::PointFiveInitParams;

use crate::callback_logger::CallbackLogger as _;
use crate::finops::pointfive::logger::core::PointFiveCore;

use super::support::{
    FakeHttp, FixedSource, client, default_params, event, logged, logger, payload,
};

fn capped(max_queue_size: usize) -> Arc<PointFiveCore> {
    let http = FakeHttp::default();
    let upload_client = client(&http, 1);
    let source = Arc::new(FixedSource {
        client: upload_client.clone(),
        calls: Arc::default(),
    });
    let mut core = PointFiveCore::new(
        PointFiveInitParams::default(),
        Some(upload_client),
        source,
        false,
    );
    core.max_queue_size = max_queue_size;
    Arc::new(core)
}

fn ids(core: &PointFiveCore) -> Vec<String> {
    core.queue
        .snapshot()
        .into_iter()
        .map(|record| record.id)
        .collect()
}

#[tokio::test]
async fn the_oldest_records_leave_when_the_queue_is_over_the_cap() {
    let core = capped(3);
    core.queue.push(payload("a"));
    core.queue.push(payload("b"));
    core.queue.push(payload("c"));
    core.queue.push(payload("d"));
    core.queue.push(payload("e"));
    let ((), text) = logged(async { core.drop_overflow() }).await;
    assert_eq!(ids(&core), vec!["c", "d", "e"]);
    assert!(text.contains("pointfive: queue over the cap"));
    assert!(text.contains("dropped=2"));
}

#[tokio::test]
async fn nothing_is_dropped_at_exactly_the_cap() {
    let core = capped(3);
    core.queue.push(payload("a"));
    core.queue.push(payload("b"));
    core.queue.push(payload("c"));
    let ((), text) = logged(async { core.drop_overflow() }).await;
    assert_eq!(ids(&core), vec!["a", "b", "c"]);
    assert!(!text.contains("pointfive: queue over the cap"));
}

#[tokio::test]
async fn a_flushing_queue_is_never_trimmed() {
    let core = capped(3);
    core.flushing.store(true, Ordering::SeqCst);
    core.queue.push(payload("a"));
    core.queue.push(payload("b"));
    core.queue.push(payload("c"));
    core.queue.push(payload("d"));
    core.queue.push(payload("e"));
    let ((), text) = logged(async { core.drop_overflow() }).await;
    assert_eq!(ids(&core), vec!["a", "b", "c", "d", "e"]);
    assert!(!text.contains("pointfive: queue over the cap"));
}

#[tokio::test]
async fn the_next_arrival_after_a_flush_trims() {
    let core = capped(3);
    core.flushing.store(true, Ordering::SeqCst);
    core.queue.push(payload("a"));
    core.queue.push(payload("b"));
    core.queue.push(payload("c"));
    core.queue.push(payload("d"));
    core.queue.push(payload("e"));
    core.drop_overflow();
    core.flushing.store(false, Ordering::SeqCst);
    core.queue.push(payload("f"));
    let ((), text) = logged(async { core.drop_overflow() }).await;
    assert_eq!(ids(&core), vec!["d", "e", "f"]);
    assert!(text.contains("dropped=3"));
}

#[tokio::test]
async fn enqueued_events_trim_the_queue() {
    let http = FakeHttp::default();
    let mut params = default_params();
    params.batch_size = 100;
    let mut logger = logger(params, client(&http, 1));
    let unshared = Arc::get_mut(&mut logger).expect("the logger is unshared");
    Arc::get_mut(&mut unshared.core)
        .expect("the core is unshared")
        .max_queue_size = 2;
    logger
        .async_log_success_event(event(payload("a")))
        .await
        .expect("the event is queued");
    logger
        .async_log_success_event(event(payload("b")))
        .await
        .expect("the event is queued");
    logger
        .async_log_success_event(event(payload("c")))
        .await
        .expect("the event is queued");
    assert_eq!(logger.core.queue.len(), 2);
    let held: Vec<String> = logger
        .core
        .queue
        .snapshot()
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(held, vec!["b".to_owned(), "c".to_owned()]);
    logger.shutdown.cancel();
}
