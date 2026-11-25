#![cfg(test)]

use std::net::SocketAddr;
use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use pyo3::exceptions::{PyNotImplementedError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PyTuple};
use pyo3_async_runtimes::tokio::get_runtime;

use super::NativeFileContentSession;
use crate::file_content::error::NativeFileContentError;
use crate::file_content::register;
use crate::file_content::stream::NativeFileStream;

const ENVIRONMENT: &str = r#"{"environment": {}}"#;

fn payload(address: &SocketAddr, streaming: bool) -> String {
    format!(
        r#"{{"file_id":"file-abc","model":"gpt-4o","api_base":"http://{address}/v1","api_key":"sk-test","stream":{streaming}}}"#
    )
}

fn provider_payload(provider: &str) -> String {
    format!(r#"{{"file_id":"file-abc","custom_llm_provider":"{provider}"}}"#)
}

fn timeout_payload(address: &SocketAddr) -> String {
    format!(
        r#"{{"file_id":"file-abc","custom_llm_provider":"openai","api_base":"http://{address}/v1","api_key":"sk-test","timeout":0.01}}"#
    )
}

fn body_of(blobs: &Bound<'_, PyAny>) -> Vec<Vec<u8>> {
    let list = blobs.cast::<PyList>().unwrap();
    list.iter()
        .map(|item| item.cast::<PyBytes>().unwrap().as_bytes().to_vec())
        .collect()
}

#[test]
fn register_adds_the_session_the_stream_and_the_error() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeFileContentSession").unwrap());
        assert!(module.hasattr("NativeFileStream").unwrap());
        assert!(module.hasattr("NativeFileContentError").unwrap());
    });
}

#[test]
fn a_malformed_settings_text_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let error = NativeFileContentSession::new("not a session")
            .err()
            .unwrap();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn a_malformed_payload_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileContentSession::new(ENVIRONMENT).unwrap();
        let error = session.content(py, "not a call", Vec::new()).err().unwrap();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn a_deadline_that_expires_fails_the_call() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("x-upstream", "yes")], "file body"),
        Duration::from_millis(200),
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileContentSession::new(ENVIRONMENT).unwrap();
        let error = session
            .content(py, &timeout_payload(&socket), Vec::new())
            .err()
            .unwrap();
        assert!(error.is_instance_of::<NativeFileContentError>(py));
        assert!(error.to_string().contains(r#""type":"APIConnectionError""#));
        assert!(error.to_string().contains(r#""status_code":500"#));
    });
    serving.finish().unwrap();
}

#[test]
fn a_plain_call_hands_out_the_status_headers_and_the_body() {
    Python::initialize();
    let serving = serve(
        reply(
            "200 OK",
            &[("x-upstream", "yes"), ("content-type", "text/plain")],
            "file body",
        ),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileContentSession::new(ENVIRONMENT).unwrap();
        let answer = session
            .content(py, &payload(&socket, false), Vec::new())
            .unwrap();
        let tuple = answer.bind(py).cast::<PyTuple>().unwrap();
        let text: String = tuple.get_item(0).unwrap().extract().unwrap();
        assert!(text.contains(r#""status":200"#));
        assert!(text.contains("x-upstream"));
        assert!(text.contains("text/plain"));
        let blobs = body_of(&tuple.get_item(1).unwrap());
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0], b"file body");
        assert!(tuple.get_item(2).unwrap().is_none());
    });
    serving.finish().unwrap();
}

#[test]
fn a_streaming_call_hands_out_the_chunks_then_stops() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("x-upstream", "yes")], "file body"),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativeFileContentSession::new(ENVIRONMENT).unwrap();
        let answer = session
            .content(py, &payload(&socket, true), Vec::new())
            .unwrap();
        let tuple = answer.bind(py).cast::<PyTuple>().unwrap();
        let text: String = tuple.get_item(0).unwrap().extract().unwrap();
        assert!(text.contains("x-upstream"));
        assert!(text.contains(r#""custom_llm_provider":"openai""#));
        assert!(body_of(&tuple.get_item(1).unwrap()).is_empty());
        let stream_object = tuple.get_item(2).unwrap();
        let class = stream_object.cast::<NativeFileStream>().unwrap();
        let stream = class.borrow();
        assert!(!stream.is_async);
        let first = stream.next_chunk(py).unwrap().unwrap();
        assert_eq!(first.bind(py).as_bytes(), b"file body");
        assert!(stream.next_chunk(py).unwrap().is_none());
        stream.close(py).unwrap();
        stream.close(py).unwrap();
        // The core closes into a sync empty source; Python swaps in its own empty iterator.
        let mut closed = get_runtime().block_on(stream.stream.lock());
        assert!(closed.sync_iter().unwrap().next_chunk().is_none());
    });
    serving.finish().unwrap();
}

#[test]
fn a_rejected_provider_is_the_family_exception_naming_bad_request() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileContentSession::new(ENVIRONMENT).unwrap();
        let error = session
            .content(py, &provider_payload("cohere"), Vec::new())
            .err()
            .unwrap();
        assert!(error.is_instance_of::<NativeFileContentError>(py));
        assert!(error.to_string().contains("BadRequestError"));
    });
}

#[test]
fn a_generic_provider_names_the_download_it_has_not_built() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileContentSession::new(ENVIRONMENT).unwrap();
        let error = session
            .content(py, &provider_payload("anthropic"), Vec::new())
            .err()
            .unwrap();
        assert!(error.is_instance_of::<PyNotImplementedError>(py));
    });
}

#[test]
fn an_xai_batch_result_names_the_download_it_has_not_built() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeFileContentSession::new(ENVIRONMENT).unwrap();
        let error = session
            .content(
                py,
                r#"{"file_id":"batch_1","custom_llm_provider":"xai"}"#,
                Vec::new(),
            )
            .err()
            .unwrap();
        assert!(error.is_instance_of::<PyNotImplementedError>(py));
    });
}
