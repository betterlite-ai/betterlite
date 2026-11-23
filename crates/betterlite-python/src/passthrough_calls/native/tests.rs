#![cfg(test)]

use std::net::SocketAddr;
use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};

use super::NativePassthroughSession;
use crate::passthrough_calls::register;
use crate::passthrough_calls::stream::NativePassthroughStream;

const ENVIRONMENT: &str = r#"{"environment": {}}"#;

fn request(address: &SocketAddr, stream: bool) -> String {
    serde_json::json!({
        "method": "POST",
        "endpoint": "chat/completions",
        "model": "vllm/my-model",
        "custom_llm_provider": "vllm",
        "api_base": format!("http://{address}/v1"),
        "json": {"model": "original", "stream": stream},
    })
    .to_string()
}

#[test]
fn register_adds_the_passthrough_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativePassthroughSession").unwrap());
        assert!(module.hasattr("NativePassthroughStream").unwrap());
        assert!(module.hasattr("NativePassthroughError").unwrap());
    });
}

#[test]
fn a_malformed_session_text_is_an_error_not_a_panic() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(NativePassthroughSession::new("not a session").is_err());
    });
}

#[test]
fn call_answers_the_status_headers_and_whole_body() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("x-upstream", "yes")], "pong"),
        Duration::ZERO,
    )
    .unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativePassthroughSession::new(ENVIRONMENT).unwrap();
        let answer = session
            .call(py, &request(&socket, false), Vec::new())
            .unwrap();
        let tuple = answer.bind(py).cast::<PyTuple>().unwrap();
        assert_eq!(tuple.get_item(0).unwrap().extract::<u16>().unwrap(), 200);
        let headers = tuple.get_item(1).unwrap();
        assert_eq!(
            headers.extract::<Vec<(String, String)>>().unwrap(),
            vec![(String::from("x-upstream"), String::from("yes"))]
        );
        let body = tuple.get_item(2).unwrap();
        assert_eq!(body.cast::<PyBytes>().unwrap().as_bytes(), b"pong");
    });
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/chat/completions HTTP/1.1");
}

#[test]
fn a_streaming_call_hands_out_the_chunks_then_stops() {
    Python::initialize();
    let serving = serve(reply("200 OK", &[], "pong"), Duration::ZERO).unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativePassthroughSession::new(ENVIRONMENT).unwrap();
        let answer = session
            .call(py, &request(&socket, true), Vec::new())
            .unwrap();
        let tuple = answer.bind(py).cast::<PyTuple>().unwrap();
        let object = tuple.get_item(2).unwrap();
        let class = object.cast::<NativePassthroughStream>().unwrap();
        let stream = class.borrow();
        let first = stream.next_chunk(py).unwrap().unwrap();
        assert_eq!(first.bind(py).as_bytes(), b"pong");
        assert!(stream.next_chunk(py).unwrap().is_none());
        stream.close(py);
    });
    serving.finish().unwrap();
}

#[test]
fn an_upstream_error_is_the_family_exception_with_the_status() {
    Python::initialize();
    let answer = reply("404 Not Found", &[], r#"{"error": "missing"}"#);
    let serving = serve(answer, Duration::ZERO).unwrap();
    let socket = serving.address;
    Python::attach(|py| {
        let session = NativePassthroughSession::new(ENVIRONMENT).unwrap();
        let error = session
            .call(py, &request(&socket, false), Vec::new())
            .unwrap_err();
        let name = error.get_type(py).name().unwrap().to_string();
        assert_eq!(name, "NativePassthroughError");
        let text = error.to_string();
        assert!(text.contains("\"status_code\":404"), "{text}");
        assert!(text.contains("missing"), "{text}");
    });
    serving.finish().unwrap();
}

#[test]
fn call_refuses_a_request_text_that_is_not_json() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativePassthroughSession::new(ENVIRONMENT).unwrap();
        let error = session.call(py, "not a call", Vec::new()).unwrap_err();
        let name = error.get_type(py).name().unwrap().to_string();
        assert_eq!(name, "ValueError");
    });
}
