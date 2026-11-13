#![cfg(test)]

use std::mem::take;
use std::sync::Arc;
use std::time::Duration;

use betterlite_lens::models::messages::{ModelRequest, Purpose};
use rstest::rstest;
use tokio::time::Instant;

use super::fixtures::{FakeApi, model_result};
use crate::api::WorkerError;
use crate::worker::LensWorker;

fn request() -> ModelRequest {
    ModelRequest {
        prompt: String::from("prompt"),
        purpose: Purpose::Extract,
    }
}

fn calls(fake: &FakeApi) -> Vec<(Instant, ModelRequest)> {
    let mut guard = fake.model_calls.lock().unwrap();
    take(&mut *guard)
}

#[tokio::test(start_paused = true)]
async fn retries_a_timeout_with_one_and_two_second_pauses() {
    let fake = FakeApi::default();
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Ok(model_result("{}")));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let answer = worker
        .model_request("/lens/worker/l/j", request())
        .await
        .unwrap();
    assert_eq!(answer.content, "{}");
    let calls = calls(&worker.api);
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[1].0.duration_since(calls[0].0),
        Duration::from_secs(1)
    );
    assert_eq!(
        calls[2].0.duration_since(calls[1].0),
        Duration::from_secs(2)
    );
}

#[rstest]
#[case::transport(WorkerError::Transport)]
#[case::rate_limited(WorkerError::Status { path: String::from("/lens/worker/l/j/model"), status: 429 })]
#[case::bad_gateway(WorkerError::Status { path: String::from("/lens/worker/l/j/model"), status: 502 })]
#[case::unavailable(WorkerError::Status { path: String::from("/lens/worker/l/j/model"), status: 503 })]
#[case::gateway_timeout(WorkerError::Status { path: String::from("/lens/worker/l/j/model"), status: 504 })]
#[tokio::test(start_paused = true)]
async fn retries_a_transient_error_once_then_returns_the_answer(#[case] error: WorkerError) {
    let fake = FakeApi::default();
    fake.model_answers.lock().unwrap().push_back(Err(error));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Ok(model_result("{}")));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let answer = worker
        .model_request("/lens/worker/l/j", request())
        .await
        .unwrap();
    assert_eq!(answer.content, "{}");
    let calls = calls(&worker.api);
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].0.duration_since(calls[0].0),
        Duration::from_secs(1)
    );
}

#[tokio::test]
async fn does_not_retry_a_status_outside_the_retry_list() {
    let fake = FakeApi::default();
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Status {
            path: String::from("/lens/worker/l/j/model"),
            status: 400,
        }));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Ok(model_result("{}")));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let result = worker.model_request("/lens/worker/l/j", request()).await;
    assert!(matches!(
        result,
        Err(WorkerError::Status { status: 400, .. })
    ));
    assert_eq!(calls(&worker.api).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn returns_the_timeout_after_three_attempts() {
    let fake = FakeApi::default();
    fake.model_answers.lock().unwrap().extend([
        Err(WorkerError::Timeout),
        Err(WorkerError::Timeout),
        Err(WorkerError::Timeout),
    ]);
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let result = worker.model_request("/lens/worker/l/j", request()).await;
    assert!(matches!(result, Err(WorkerError::Timeout)));
    let calls = calls(&worker.api);
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[1].0.duration_since(calls[0].0),
        Duration::from_secs(1)
    );
    assert_eq!(
        calls[2].0.duration_since(calls[1].0),
        Duration::from_secs(2)
    );
}

#[tokio::test(start_paused = true)]
async fn returns_the_last_error_when_it_stops_being_transient() {
    let fake = FakeApi::default();
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Timeout));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Err(WorkerError::Status {
            path: String::from("/lens/worker/l/j/model"),
            status: 400,
        }));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let result = worker.model_request("/lens/worker/l/j", request()).await;
    assert!(matches!(
        result,
        Err(WorkerError::Status { status: 400, .. })
    ));
    assert_eq!(calls(&worker.api).len(), 3);
}

#[rstest]
#[case::decode(WorkerError::Decode(String::from("bad json")))]
#[case::storage(WorkerError::Storage(String::from("sqlite is full")))]
#[tokio::test]
async fn does_not_retry_a_local_failure(#[case] error: WorkerError) {
    let expected = error.kind();
    let fake = FakeApi::default();
    fake.model_answers.lock().unwrap().push_back(Err(error));
    fake.model_answers
        .lock()
        .unwrap()
        .push_back(Ok(model_result("{}")));
    let worker = LensWorker {
        api: Arc::new(fake),
    };
    let failure = worker
        .model_request("/lens/worker/l/j", request())
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), expected);
    assert_eq!(calls(&worker.api).len(), 1);
}
