#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_service_events::types::ServiceType;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use chrono::TimeDelta;
use uuid::Uuid;

use super::super::hooks::{CallIdGenerator, ServiceEventHooks};
use super::support::{Broken, FailingSink, RecordingSink, fixed_clock};

fn call_ids() -> CallIdGenerator {
    Arc::new(|| Uuid::new_v4().to_string())
}

fn hooks(sink: Arc<RecordingSink>) -> ServiceEventHooks {
    ServiceEventHooks {
        sink,
        clock: Arc::new(fixed_clock()),
        call_ids: call_ids(),
    }
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
    let events = sink.events();
    assert_eq!(events.len(), 1);
    let logged = &events[0];
    assert_eq!(logged.payload.service, ServiceType::Redis);
    assert_eq!(logged.payload.call_type, "async_get_cache");
    assert_eq!(logged.payload.duration, 1.5);
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
    let events = sink.events();
    assert_ne!(events[0].call_id, events[1].call_id);
}

#[test]
fn the_call_id_comes_from_the_injected_generator() {
    let sink = Arc::new(RecordingSink::new());
    let hooks = ServiceEventHooks {
        sink: Arc::clone(&sink),
        clock: Arc::new(fixed_clock()),
        call_ids: Arc::new(|| "call-from-the-test".to_owned()),
    };
    hooks.success(&event());
    let events = sink.events();
    assert_eq!(events[0].call_id, "call-from-the-test");
}

#[test]
fn failure_sends_the_error_text() {
    let sink = Arc::new(RecordingSink::new());
    hooks(Arc::clone(&sink)).failure(&event(), &Broken);
    let events = sink.events();
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
    assert_eq!(sink.events().len(), 1);
}

#[test]
fn a_failing_sink_does_not_panic() {
    let hooks = ServiceEventHooks {
        sink: Arc::new(FailingSink),
        clock: Arc::new(fixed_clock()),
        call_ids: call_ids(),
    };
    hooks.success(&event());
    hooks.failure(&event(), &Broken);
}
