#![cfg(test)]

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::call_setup::RequestMap;
use crate::router_call_contract::GenericCallExecutor;
use crate::router_call_contract::GenericOutcome;
use crate::router_call_contract::GenericRequest;
use crate::router_call_contract::error::GenericError;
use crate::router_call_contract::executors::GenericExecutors;

type RecordedCall = (String, Vec<(String, Json)>);

struct RecordingExecutor {
    calls: Mutex<Vec<RecordedCall>>,
}

impl RecordingExecutor {
    fn call_log(&self) -> Vec<RecordedCall> {
        self.calls.lock().expect("the call log is free").clone()
    }
}

#[async_trait]
impl GenericCallExecutor for RecordingExecutor {
    async fn call(&self, request: GenericRequest) -> Result<GenericOutcome, GenericError> {
        let arguments: Vec<(String, Json)> = request
            .args
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.calls
            .lock()
            .expect("the call log is free")
            .push((request.call_type.clone(), arguments));
        Ok(GenericOutcome::Json(Json::Text(request.call_type)))
    }
}

fn recording() -> (Arc<RecordingExecutor>, Arc<dyn GenericCallExecutor>) {
    let handle = Arc::new(RecordingExecutor {
        calls: Mutex::new(Vec::new()),
    });
    let executor: Arc<dyn GenericCallExecutor> = Arc::<RecordingExecutor>::clone(&handle);
    (handle, executor)
}

#[tokio::test]
async fn executor_lookup_returns_the_one_registered_under_the_name() {
    let (first, first_executor) = recording();
    let (second, second_executor) = recording();
    let executors = GenericExecutors::new(vec![
        (String::from("aembedding"), first_executor),
        (String::from("acreate_batch"), second_executor),
    ])
    .expect("the call types differ");

    let executor = executors
        .executor("acreate_batch")
        .expect("the call type is registered");
    let outcome = executor
        .call(GenericRequest {
            call_type: String::from("acreate_batch"),
            args: RequestMap::new(),
        })
        .await
        .expect("the fake answers");

    let GenericOutcome::Json(Json::Text(call_type)) = outcome else {
        panic!("the fake answers with the call type");
    };
    assert_eq!(call_type, "acreate_batch");
    let calls = second.call_log();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls.first().expect("the fake saw the call").0,
        "acreate_batch"
    );
    assert!(first.call_log().is_empty());
}

#[tokio::test]
async fn executor_receives_the_arguments_in_order_with_a_later_key_winning() {
    let (recorder, executor) = recording();
    let executors = GenericExecutors::new(vec![(String::from("aembedding"), executor)])
        .expect("the call type appears once");
    let mut args = RequestMap::new();
    args.insert(String::from("model"), Json::Text(String::from("first")));
    args.insert(String::from("input"), Json::Text(String::from("hello")));
    args.insert(String::from("model"), Json::Text(String::from("second")));

    let outcome = executors
        .executor("aembedding")
        .expect("the call type is registered")
        .call(GenericRequest {
            call_type: String::from("aembedding"),
            args,
        })
        .await
        .expect("the fake answers");
    let GenericOutcome::Json(Json::Text(call_type)) = outcome else {
        panic!("the fake answers with the call type");
    };
    assert_eq!(call_type, "aembedding");

    let calls = recorder.call_log();
    let call = calls.first().expect("the fake saw the call");
    assert_eq!(
        call.1,
        [
            (String::from("model"), Json::Text(String::from("second"))),
            (String::from("input"), Json::Text(String::from("hello"))),
        ]
    );
}

#[test]
fn executor_unknown_name_answers_unknown_call_type() {
    let (_known, executor) = recording();
    let executors = GenericExecutors::new(vec![(String::from("aembedding"), executor)])
        .expect("the call type appears once");

    let Err(error) = executors.executor("avideo_status") else {
        panic!("an unregistered call type is refused");
    };
    let GenericError::UnknownCallType(call_type) = error else {
        panic!("an unregistered call type names itself");
    };
    assert_eq!(call_type, "avideo_status");
}

#[test]
fn executor_duplicate_call_type_is_refused_with_its_name() {
    let (_first_handle, first) = recording();
    let (_second_handle, second) = recording();
    let (_third_handle, third) = recording();

    let Err(error) = GenericExecutors::new(vec![
        (String::from("ocr"), first),
        (String::from("aembedding"), second),
        (String::from("ocr"), third),
    ]) else {
        panic!("a repeated call type is refused");
    };
    assert_eq!(error.0, "ocr");
    assert_eq!(error.to_string(), "the call type ocr has two executors");
}
