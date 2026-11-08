use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;

#[derive(Debug, thiserror::Error)]
#[error("the redis command failed")]
struct TestError;

#[derive(Clone, Default)]
struct GateLog {
    exits: Arc<Mutex<Vec<u64>>>,
    failures: Arc<Mutex<Vec<String>>>,
    swallowed: Arc<Mutex<Vec<String>>>,
}

struct TestGate {
    log: GateLog,
}

impl BreakerGate for TestGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        if name == "open" {
            return Err(BreakerOpen { name });
        }
        Ok(BreakerAdmission::from(7))
    }

    fn exit(&self, admission: BreakerAdmission) {
        self.log
            .exits
            .lock()
            .expect("the exit log")
            .push(*admission.as_ref());
    }

    fn record_failure(&self, error: &(dyn Error + Send + Sync)) {
        self.log
            .failures
            .lock()
            .expect("the failure log")
            .push(error.to_string());
    }

    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn Error + Send + Sync)) {
        self.log
            .swallowed
            .lock()
            .expect("the swallowed log")
            .push(format!("{}:{error}", *admission.as_ref()));
    }
}

#[derive(Clone, Default)]
struct HookLog {
    successes: Arc<Mutex<Vec<String>>>,
    failures: Arc<Mutex<Vec<String>>>,
}

struct TestHooks {
    log: HookLog,
}

impl ServiceHooks for TestHooks {
    fn success(&self, event: &ServiceEvent) {
        self.log
            .successes
            .lock()
            .expect("the success log")
            .push(format!("{}:{:?}", event.call_type, event.key));
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log
            .failures
            .lock()
            .expect("the failure log")
            .push(format!("{}:{error}", event.call_type));
    }
}

#[test]
fn a_breaker_gate_runs_as_a_trait_object() {
    let log = GateLog::default();
    let gate: Arc<dyn BreakerGate> = Arc::new(TestGate { log: log.clone() });
    let admission = gate.enter("redis_cache").expect("an admission");
    assert_eq!(*admission.as_ref(), 7);
    gate.record_failure(&TestError);
    gate.swallowed(&admission, &TestError);
    gate.exit(admission);
    assert_eq!(*log.exits.lock().expect("the exit log"), vec![7]);
    assert_eq!(
        *log.failures.lock().expect("the failure log"),
        vec!["the redis command failed".to_owned()]
    );
    assert_eq!(
        *log.swallowed.lock().expect("the swallowed log"),
        vec!["7:the redis command failed".to_owned()]
    );
}

#[test]
fn an_open_breaker_names_the_breaker() {
    let gate: Arc<dyn BreakerGate> = Arc::new(TestGate {
        log: GateLog::default(),
    });
    let error = gate.enter("open").expect_err("the breaker is open");
    assert_eq!(
        error.to_string(),
        "Redis circuit breaker is open, skipping open"
    );
}

#[test]
fn the_service_hooks_run_as_a_trait_object() {
    let log = HookLog::default();
    let hooks: Arc<dyn ServiceHooks> = Arc::new(TestHooks { log: log.clone() });
    let event = ServiceEvent {
        call_type: "get_cache",
        duration: Duration::from_millis(5),
        key: Some("cache:key".to_owned()),
    };
    assert_eq!(event.call_type, "get_cache");
    assert_eq!(event.duration, Duration::from_millis(5));
    assert_eq!(event.key.as_deref(), Some("cache:key"));
    hooks.success(&event);
    hooks.failure(&event, &TestError);
    assert_eq!(
        *log.successes.lock().expect("the success log"),
        vec!["get_cache:Some(\"cache:key\")".to_owned()]
    );
    assert_eq!(
        *log.failures.lock().expect("the failure log"),
        vec!["get_cache:the redis command failed".to_owned()]
    );
}
