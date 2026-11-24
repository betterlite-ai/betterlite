#![cfg(test)]

use betterlite_core::api::assistant_messages::service::Operation;
use betterlite_core::api::assistant_run_sdk::error::AssistantRunError;
use betterlite_sdk::call_wrapper::error::class_name;
use pyo3_async_runtimes::tokio::get_runtime;

use super::Session;
use crate::assistant_runs::convert::{MessageCall, RunCall, read};

const RUN: &str = r#"{"custom_llm_provider":"bedrock","thread_id":"thread_1",
    "assistant_id":"asst_1"}"#;

#[test]
fn run_for_an_unsupported_provider_answers_an_error() {
    let session = Session::build("{}").unwrap();
    let call: RunCall = read("assistant run request", RUN).unwrap();
    let outcome = get_runtime().block_on(session.run_sdk().run_thread(call.into_call()));
    assert!(matches!(
        outcome,
        Err(AssistantRunError::Unsupported { .. })
    ));
}

#[test]
fn stream_for_an_unsupported_provider_answers_an_error() {
    let session = Session::build("{}").unwrap();
    let call: RunCall = read("assistant run request", RUN).unwrap();
    let outcome = session.run_sdk().run_thread_stream(call.into_call());
    assert!(matches!(
        outcome,
        Err(AssistantRunError::Unsupported { .. })
    ));
}

#[test]
fn list_for_an_unsupported_provider_answers_a_bad_request() {
    let session = Session::build("{}").unwrap();
    let text = r#"{"custom_llm_provider":"bedrock","thread_id":"thread_1"}"#;
    let call: MessageCall = read("assistant get messages request", text).unwrap();
    let failure = get_runtime()
        .block_on(session.message(&call, Operation::List, false))
        .unwrap_err();
    assert_eq!(class_name(&failure.class), "BadRequestError");
}
