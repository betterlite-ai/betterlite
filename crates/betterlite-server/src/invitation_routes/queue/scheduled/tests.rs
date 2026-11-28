#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::error::ErrorClass;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_router::request_scheduler::error::SchedulerError;
use betterlite_router::request_scheduler::schedule::ScheduledCall as _;

use super::{QueuedCompletion, scheduler_exception};
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::invitation_routes::state::{QueueAnswer, QueueCompletion};

struct FixedCompletion {
    healthy: usize,
    models: Mutex<Vec<String>>,
}

impl FixedCompletion {
    fn new(healthy: usize) -> Self {
        Self {
            healthy,
            models: Mutex::new(Vec::new()),
        }
    }

    fn models(&self) -> Vec<String> {
        self.models.lock().expect("the completion lock").clone()
    }
}

#[async_trait]
impl QueueCompletion for FixedCompletion {
    fn healthy_deployments(&self, model: &str) -> usize {
        self.models
            .lock()
            .expect("the completion lock")
            .push(model.to_owned());
        self.healthy
    }

    async fn complete(
        &self,
        _caller: &Principal,
        _data: &JsonMap,
        _stream: bool,
    ) -> Result<QueueAnswer, Box<ErrorClass>> {
        panic!("this test reads the health count only")
    }
}

fn principal() -> Principal {
    master_caller().principal
}

fn request(model: &str) -> JsonMap {
    let mut data = JsonMap::new();
    data.insert("model".to_owned(), JsonValue::String(model.to_owned()));
    data
}

fn adapter<'call>(
    completion: &'call FixedCompletion,
    caller: &'call Principal,
    data: &'call JsonMap,
) -> QueuedCompletion<'call> {
    QueuedCompletion {
        completion,
        caller,
        data,
        stream: false,
        source: Mutex::new(None),
    }
}

#[test]
fn queued_completion_reads_the_health_count_and_the_model() {
    let completion = FixedCompletion::new(1);
    let caller = principal();
    let data = request("gpt-test");
    let call = adapter(&completion, &caller, &data);
    assert_eq!(call.healthy_deployments(), 1);
    assert_eq!(completion.models(), vec!["gpt-test".to_owned()]);
}

#[test]
fn queued_completion_reports_no_healthy_deployment() {
    let completion = FixedCompletion::new(0);
    let caller = principal();
    let data = request("gpt-test");
    let call = adapter(&completion, &caller, &data);
    assert_eq!(call.healthy_deployments(), 0);
}

#[test]
fn a_full_scheduler_queue_answers_429() {
    let error = SchedulerError::QueueFull {
        model: "gpt-test".to_owned(),
        limit: 1024,
    };
    let exception = scheduler_exception(&error);
    assert_eq!(exception.code, "429");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(
        exception.message,
        "the scheduler queue for gpt-test holds the maximum of 1024 pending requests"
    );
}
