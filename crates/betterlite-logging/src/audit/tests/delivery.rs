#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::super::callbacks::NamedAuditCallbacks;
use super::super::delivery::{AuditCallback, AuditDelivery, DispatchReport};
use super::support::{
    FailingBuilder, FakeResolver, PlainLogger, RecordingLogger, capture, payload, settings,
};
use crate::callback_logger::CallbackLogger as _;
use crate::worker::LoggingWorker;

fn delivery(queue: usize) -> AuditDelivery {
    AuditDelivery {
        named: NamedAuditCallbacks {
            resolver: Arc::new(FakeResolver::new(Vec::new(), false)),
            dedicated: Arc::new(FailingBuilder),
            cache: Mutex::new(BTreeMap::new()),
        },
        worker: LoggingWorker::start(settings(queue)).expect("the worker starts"),
    }
}

fn loggers(entries: &[(&str, bool)]) -> (Vec<AuditCallback>, Vec<Arc<RecordingLogger>>) {
    let mut callbacks = Vec::new();
    let mut recorders = Vec::new();
    for (name, fails) in entries {
        let recorder = Arc::new(RecordingLogger::new(name, *fails));
        callbacks.push(Arc::clone(&recorder));
        recorders.push(recorder);
    }
    (callbacks, recorders)
}

#[tokio::test]
async fn dispatch_submits_every_callback_and_flush_waits_for_delivery() {
    let delivery = delivery(8);
    let (callbacks, recorders) = loggers(&[("first", false), ("second", false)]);
    let audit = payload();
    let report = delivery.dispatch(&callbacks, &audit);
    assert_eq!(
        report,
        DispatchReport {
            submitted: 2,
            rejected: 0
        }
    );
    delivery.worker.flush().await;
    for recorder in &recorders {
        assert_eq!(*recorder.delivered.lock().unwrap(), vec![audit.clone()]);
    }
}

#[tokio::test]
async fn a_failing_callback_does_not_cancel_the_other() {
    let delivery = delivery(8);
    let (callbacks, recorders) = loggers(&[("bad", true), ("good", false)]);
    let report = delivery.dispatch(&callbacks, &payload());
    assert_eq!(report.submitted, 2);
    let flushed = delivery.worker.flush().await;
    assert_eq!(flushed.failed, 1);
    assert_eq!(flushed.completed, 1);
    assert!(recorders[0].delivered.lock().unwrap().is_empty());
    assert_eq!(recorders[1].delivered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_full_queue_rejects_admission_without_waiting() {
    let (captured, guard) = capture();
    let delivery = delivery(1);
    let (callbacks, recorders) = loggers(&[("first", false), ("second", false)]);
    let report = delivery.dispatch(&callbacks, &payload());
    drop(guard);
    assert_eq!(
        report,
        DispatchReport {
            submitted: 1,
            rejected: 1
        }
    );
    assert!(
        captured
            .text()
            .contains("the audit log callback was rejected")
    );
    assert!(recorders[0].delivered.lock().unwrap().is_empty());
    assert!(recorders[1].delivered.lock().unwrap().is_empty());
    drop(delivery);
}

#[tokio::test]
async fn shutdown_drains_accepted_callbacks() {
    let delivery = delivery(8);
    let (callbacks, recorders) = loggers(&[("otel", false)]);
    delivery.dispatch(&callbacks, &payload());
    let report = delivery.shutdown().await;
    assert!(report.drained);
    assert_eq!(report.completed, 1);
    assert_eq!(recorders[0].delivered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_logger_without_the_override_ignores_audit_events() {
    let logger = PlainLogger;
    logger
        .async_log_audit_log_event(&payload())
        .await
        .expect("the default override accepts and drops the event");
}
