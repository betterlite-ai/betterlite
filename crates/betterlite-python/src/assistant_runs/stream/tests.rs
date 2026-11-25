#![cfg(test)]

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;

use super::{advance, deferred};
use crate::assistant_runs::convert::{RunCall, read};
use crate::assistant_runs::session::Session;

// The api_base is a refused port, so a stream that was not closed fails its first read instead of answering None.
const SETTINGS: &str = r#"{"api_base":"http://127.0.0.1:1","api_key":"sk-test"}"#;
const RUN: &str =
    r#"{"custom_llm_provider":"openai","thread_id":"thread_1","assistant_id":"asst_1"}"#;

#[test]
fn close_ends_a_stream_that_was_never_read() {
    let session = Session::build(SETTINGS).unwrap();
    let call: RunCall = read("assistant run request", RUN).unwrap();
    let source = session
        .run_sdk()
        .run_thread_stream(call.into_call())
        .unwrap();
    let stream = deferred(source, "openai".to_owned(), Arc::clone(&session.redactor));
    Python::initialize();
    Python::attach(|py| stream.close(py));
    let next = get_runtime()
        .block_on(advance(&stream.state, "openai", session.redactor.as_ref()))
        .unwrap();
    assert_eq!(next, None);
}
