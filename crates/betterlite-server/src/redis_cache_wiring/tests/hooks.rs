#![cfg(test)]

use std::sync::Arc;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

use betterlite_logging::logging_service_events::types::ServiceType;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use chrono::TimeDelta;
use tracing::Level;
use uuid::Uuid;

use super::super::hooks::{CallIdGenerator, EVENTS_QUEUED, ServiceEventHooks};
use super::support::{
    BlockedSink, Broken, FailingSink, RecordingSink, SINK_WAIT, capture_logs, emit_successes,
    fixed_clock,
};

fn call_ids() -> CallIdGenerator {
    Arc::new(|| Uuid::new_v4().to_string())
}

fn hooks(sink: Arc<RecordingSink>) -> ServiceEventHooks {
    ServiceEventHooks::start(sink, Arc::new(fixed_clock()), call_ids())
        .expect("the service event worker starts")
}

fn event() -> ServiceEvent {
    ServiceEvent {
        call_type: "async_get_cache",
        duration: Duration::from_millis(1500),
        key: Some("cache:key".to_owned()),
    }
}

#[test]
fn success_sends_a_redis_event() {
    let sink = Arc::new(RecordingSink::new());
    let clock = fixed_clock();
    hooks(Arc::clone(&sink)).success(&event());
    let events = sink.wait_for_events(1);
    assert_eq!(events.len(), 1);
    let logged = &events[0];
    assert_eq!(logged.payload.service, ServiceType::Redis);
    assert_eq!(logged.payload.call_type, "async_get_cache");
    assert_eq!(logged.payload.duration.to_bits(), 1.5_f64.to_bits());
    assert!(!logged.payload.is_error);
    assert_eq!(logged.payload.error, None);
    assert_eq!(logged.payload.caller, None);
    assert_eq!(
        logged.payload.event_metadata, None,
        "B012's event metadata carries gauge fields only"
    );
    assert_eq!(logged.end_time, clock.now);
    assert_eq!(logged.start_time, clock.now - TimeDelta::milliseconds(1500));
    Uuid::parse_str(&logged.call_id).expect("the call id is a uuid");
}

#[test]
fn every_event_gets_its_own_call_id() {
    let sink = Arc::new(RecordingSink::new());
    let hooks = hooks(Arc::clone(&sink));
    hooks.success(&event());
    hooks.success(&event());
    let events = sink.wait_for_events(2);
    assert_ne!(events[0].call_id, events[1].call_id);
}

#[test]
fn the_call_id_comes_from_the_injected_generator() {
    let sink = Arc::new(RecordingSink::new());
    let hooks = ServiceEventHooks::start(
        Arc::clone(&sink),
        Arc::new(fixed_clock()),
        Arc::new(|| "call-from-the-test".to_owned()),
    )
    .expect("the service event worker starts");
    hooks.success(&event());
    let events = sink.wait_for_events(1);
    assert_eq!(events[0].call_id, "call-from-the-test");
}

#[test]
fn failure_sends_the_error_text() {
    let sink = Arc::new(RecordingSink::new());
    hooks(Arc::clone(&sink)).failure(&event(), &Broken);
    let events = sink.wait_for_events(1);
    assert_eq!(events.len(), 1);
    let logged = &events[0];
    assert!(logged.payload.is_error);
    assert_eq!(logged.payload.error.as_deref(), Some("the cache broke"));
    assert_eq!(logged.payload.service, ServiceType::Redis);
    assert_eq!(logged.payload.call_type, "async_get_cache");
}

#[test]
fn the_hooks_need_no_runtime() {
    // a plain test thread has no tokio runtime, the way a sync cache method calls these
    let sink = Arc::new(RecordingSink::new());
    hooks(Arc::clone(&sink)).success(&event());
    assert_eq!(sink.wait_for_events(1).len(), 1);
}

#[test]
fn a_failing_sink_does_not_panic() {
    let (calls, arrived) = sync_channel(2);
    let hooks = ServiceEventHooks::start(
        Arc::new(FailingSink { calls }),
        Arc::new(fixed_clock()),
        call_ids(),
    )
    .expect("the service event worker starts");
    hooks.success(&event());
    hooks.failure(&event(), &Broken);
    arrived
        .recv_timeout(SINK_WAIT)
        .expect("the first event reached the failing sink");
    arrived
        .recv_timeout(SINK_WAIT)
        .expect("the second event reached the failing sink");
}

#[test]
fn a_blocked_sink_does_not_delay_either_hook() {
    let (entered, entered_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    let sink = Arc::new(BlockedSink::new(entered, release_rx));
    let hooks = Arc::new(
        ServiceEventHooks::start(sink, Arc::new(fixed_clock()), call_ids())
            .expect("the service event worker starts"),
    );
    let (success_done, success_done_rx) = sync_channel(1);
    let (failure_done, failure_done_rx) = sync_channel(1);
    thread::scope(|scope| {
        let success_hooks = Arc::clone(&hooks);
        scope.spawn(move || {
            success_hooks.success(&event());
            let _signalled = success_done.try_send(());
        });
        let announced = entered_rx.recv_timeout(SINK_WAIT).is_ok();
        let success_returned = success_done_rx.recv_timeout(SINK_WAIT).is_ok();
        let failure_hooks = Arc::clone(&hooks);
        scope.spawn(move || {
            failure_hooks.failure(&event(), &Broken);
            let _signalled = failure_done.try_send(());
        });
        let failure_returned = failure_done_rx.recv_timeout(SINK_WAIT).is_ok();
        let _released = release.try_send(());
        assert!(announced, "the worker thread reached the blocked sink");
        assert!(
            success_returned,
            "the success hook returns while the sink blocks"
        );
        assert!(
            failure_returned,
            "a plain thread's failure hook returns while the sink blocks"
        );
    });
    drop(hooks);
}

#[test]
fn an_event_over_the_queue_capacity_is_dropped_with_a_warning() {
    let (entered, entered_rx) = sync_channel(1);
    let (release, release_rx) = sync_channel(1);
    let sink = Arc::new(BlockedSink::new(entered, release_rx));
    let hooks = ServiceEventHooks::start(sink, Arc::new(fixed_clock()), call_ids())
        .expect("the service event worker starts");
    hooks.success(&event());
    entered_rx
        .recv_timeout(SINK_WAIT)
        .expect("the worker is parked in the sink");
    let lines = capture_logs(|| emit_successes(&hooks, &event(), EVENTS_QUEUED + 1));
    let _released = release.try_send(());
    let warnings: Vec<&str> = lines
        .iter()
        .filter(|line| line.level == Level::WARN)
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(
        warnings,
        ["the redis service event was dropped"],
        "the one event over the queue's capacity is dropped with a warning"
    );
}
