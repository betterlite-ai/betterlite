#![cfg(test)]

use std::ffi::CStr;
use std::net::SocketAddr;
use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::serve;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyModule};
use pyo3::{Py, Python};

use super::{NativeVideoEditSession, is_litellm_owned_kwarg};
use crate::video_edits::error::NativeVideoEditError;
use crate::video_edits::register;

const CHARACTER: &str = r#"{"id":"char_test","object":"character","created_at":7,"name":"hero"}"#;
const VIDEO: &str = r#"{"id":"vid_test","object":"video","status":"queued"}"#;
const UNAUTHORIZED: &str =
    r#"{"error":{"message":"Incorrect API key provided: sk-test","type":"invalid_request_error"}}"#;
const INVOKE: &CStr = c"import asyncio, inspect
async def invoke():
    result = getattr(session, method)(payload, blobs)
    if inspect.isawaitable(result):
        result = await result
    return result
answer = asyncio.run(invoke())
";

fn new_session(py: Python<'_>) -> Py<NativeVideoEditSession> {
    let session = NativeVideoEditSession::new(r#"{"environment":{}}"#).unwrap();
    Py::new(py, session).unwrap()
}

fn invoke(method: &str, payload: &str, blobs: Vec<Vec<u8>>) -> String {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "video_edit_test").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeVideoEditError").unwrap());
        let session = module
            .getattr("NativeVideoEditSession")
            .unwrap()
            .call1((r#"{"environment":{}}"#,))
            .unwrap();
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("method", method).unwrap();
        globals.set_item("payload", payload).unwrap();
        globals.set_item("blobs", blobs).unwrap();
        py.run(INVOKE, Some(&globals), None).unwrap();
        let answer = globals.get_item("answer").unwrap().unwrap();
        answer.extract().unwrap()
    })
}

fn payload(address: &SocketAddr, arguments: &str) -> String {
    format!(
        r#"{{"settings":{{}},"arguments":{{"api_base":"http://{address}/v1","api_key":"sk-test",{arguments}}}}}"#
    )
}

fn call(method: &str, arguments: &str, response: &str, blobs: Vec<Vec<u8>>) -> (String, Request) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], response),
        Duration::ZERO,
    )
    .unwrap();
    let request_payload = payload(&serving.address, arguments);
    let answer = invoke(method, &request_payload, blobs);
    let request = serving.finish().unwrap();
    (answer, request)
}

fn failure_message(method: &str, payload: &str) -> String {
    Python::initialize();
    Python::attach(|py| {
        let session = new_session(py);
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("method", method).unwrap();
        globals.set_item("payload", payload).unwrap();
        globals.set_item("blobs", Vec::<Vec<u8>>::new()).unwrap();
        // The async twins need a running loop, so run both through asyncio.run.
        let error = py.run(INVOKE, Some(&globals), None).unwrap_err();
        assert!(error.is_instance_of::<NativeVideoEditError>(py), "{error}");
        error.value(py).to_string()
    })
}

// one roundtrip: the arguments and answer the call is expected to make of them
struct Expectation<'text> {
    arguments: &'text str,
    response: &'text str,
    blob: Option<&'text [u8]>,
    request_start: &'text str,
    head_fragments: &'text [&'text str],
    body_fragments: &'text [&'text str],
    answer_fragments: &'text [&'text str],
}

fn check(method: &str, expected: &Expectation<'_>) {
    let blobs = expected
        .blob
        .map_or_else(Vec::new, |bytes| vec![bytes.to_vec()]);
    let (answer, request) = call(method, expected.arguments, expected.response, blobs);
    assert!(
        request.line.starts_with(expected.request_start),
        "{}",
        request.line
    );
    for &fragment in expected.head_fragments {
        assert!(
            request.head.to_lowercase().contains(fragment),
            "{}",
            request.head
        );
    }
    for &fragment in expected.body_fragments {
        assert!(request.body.contains(fragment), "{}", request.body);
    }
    for &fragment in expected.answer_fragments {
        assert!(answer.contains(fragment), "{answer}");
    }
}

const MEDIA: &[u8] = b"video bytes";
const CREATE: Expectation<'static> = Expectation {
    arguments: r#""name":"hero","video":"https://cdn.test/a.mp4","custom_llm_provider":"openai""#,
    response: CHARACTER,
    blob: None,
    request_start: "POST /v1/videos/characters",
    head_fragments: &["authorization: bearer sk-test"],
    body_fragments: &["hero", "a.mp4"],
    answer_fragments: &[r#""id":"char_test""#, r#""name":"hero""#],
};
const CREATE_FILE: Expectation<'static> = Expectation {
    arguments: r#""name":"hero","video":{"$file":{"name":"clip.mov","blob":0,"content_type":null}},"custom_llm_provider":"openai""#,
    response: CHARACTER,
    blob: Some(MEDIA),
    request_start: "POST /v1/videos/characters",
    head_fragments: &[],
    body_fragments: &["clip.mov", "video bytes"],
    answer_fragments: &[r#""id":"char_test""#],
};
const GET: Expectation<'static> = Expectation {
    arguments: r#""character_id":"char_test","custom_llm_provider":"openai""#,
    response: CHARACTER,
    blob: None,
    request_start: "GET /v1/videos/characters/char_test",
    head_fragments: &[],
    body_fragments: &[],
    answer_fragments: &[r#""name":"hero""#],
};
const EDIT: Expectation<'static> = Expectation {
    arguments: r#""video_id":"vid-1","prompt":"add rain","custom_llm_provider":"openai""#,
    response: VIDEO,
    blob: None,
    request_start: "POST /v1/videos/edits",
    head_fragments: &[],
    body_fragments: &["add rain", "vid-1"],
    answer_fragments: &[r#""status":"queued""#],
};
const EDIT_FILE: Expectation<'static> = Expectation {
    arguments: r#""video_id":"vid-1","prompt":"add rain","video":{"$file":{"name":"clip.mp4","blob":0,"content_type":null}},"custom_llm_provider":"openai""#,
    response: VIDEO,
    blob: Some(MEDIA),
    request_start: "POST /v1/videos/edits",
    head_fragments: &[],
    body_fragments: &["clip.mp4", "add rain"],
    answer_fragments: &[r#""status":"queued""#],
};
const EXTENSION: Expectation<'static> = Expectation {
    arguments: r#""video_id":"vid-1","prompt":"more","seconds":"8","custom_llm_provider":"openai""#,
    response: VIDEO,
    blob: None,
    request_start: "POST /v1/videos/extensions",
    head_fragments: &[],
    body_fragments: &[r#""seconds":"8""#],
    answer_fragments: &[r#""status":"queued""#],
};

fn check_failed(method: &str, arguments: &str) {
    let serving = serve(
        reply(
            "401 Unauthorized",
            &[("content-type", "application/json")],
            UNAUTHORIZED,
        ),
        Duration::ZERO,
    )
    .unwrap();
    let message = failure_message(method, &payload(&serving.address, arguments));
    assert!(
        message.contains(r#""type":"AuthenticationError""#),
        "{message}"
    );
    assert!(message.contains(r#""status_code":401"#), "{message}");
    assert!(message.contains("Incorrect API key provided"), "{message}");
    serving.finish().unwrap();
}

fn check_refused(method: &str, arguments: &str) {
    let address: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let message = failure_message(method, &payload(&address, arguments));
    assert!(message.contains(r#""type":"BadRequestError""#), "{message}");
    assert!(message.contains(r#""status_code":400"#), "{message}");
    assert!(message.contains("not supported for"), "{message}");
}

fn check_mock(method: &str, arguments: &str, expected: &str) {
    let address: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let answer = invoke(method, &payload(&address, arguments), Vec::new());
    assert!(answer.contains(expected), "{answer}");
}

#[test]
fn character_create_roundtrip() {
    check("create_character", &CREATE);
    check("acreate_character", &CREATE);
}

#[test]
fn character_create_file_roundtrip() {
    check("create_character", &CREATE_FILE);
    check("acreate_character", &CREATE_FILE);
}

#[test]
fn character_get_roundtrip() {
    check("get_character", &GET);
    check("aget_character", &GET);
}

#[test]
fn edit_roundtrip() {
    check("edit", &EDIT);
    check("aedit", &EDIT);
}

#[test]
fn edit_file_roundtrip() {
    check("edit", &EDIT_FILE);
    check("aedit", &EDIT_FILE);
}

#[test]
fn extension_roundtrip() {
    check("extension", &EXTENSION);
    check("aextension", &EXTENSION);
}

#[test]
fn character_create_error() {
    let arguments = r#""name":"hero","custom_llm_provider":"openai""#;
    check_failed("create_character", arguments);
    check_failed("acreate_character", arguments);
}

#[test]
fn character_get_error() {
    let arguments = r#""character_id":"char_test","custom_llm_provider":"openai""#;
    check_failed("get_character", arguments);
    check_failed("aget_character", arguments);
}

#[test]
fn edit_error() {
    let arguments = r#""video_id":"vid-1","prompt":"p","custom_llm_provider":"openai""#;
    check_failed("edit", arguments);
    check_failed("aedit", arguments);
}

#[test]
fn extension_error() {
    let arguments =
        r#""video_id":"vid-1","prompt":"p","seconds":"8","custom_llm_provider":"openai""#;
    check_failed("extension", arguments);
    check_failed("aextension", arguments);
}

#[test]
fn character_mock() {
    check_mock(
        "create_character",
        r#""name":"hero","mock_response":{"id":"mock_char","object":"character","created_at":1,"name":"mock"}"#,
        r#""id":"mock_char""#,
    );
}

#[test]
fn edit_mock() {
    check_mock(
        "edit",
        r#""video_id":"vid-1","prompt":"p","mock_response":{"id":"vid_mock","object":"video","status":"queued"}"#,
        r#""id":"vid_mock""#,
    );
}

#[test]
fn character_provider_refusal() {
    let arguments = r#""name":"hero","custom_llm_provider":"runwayml""#;
    check_refused("create_character", arguments);
    let lookup = r#""character_id":"char_test","custom_llm_provider":"runwayml""#;
    check_refused("get_character", lookup);
}

#[test]
fn edit_provider_refusal() {
    let arguments = r#""video_id":"vid-1","prompt":"p","custom_llm_provider":"gemini""#;
    check_refused("edit", arguments);
}

#[test]
fn extension_provider_refusal() {
    let arguments =
        r#""video_id":"vid-1","prompt":"p","seconds":"8","custom_llm_provider":"gemini""#;
    check_refused("extension", arguments);
}

#[test]
fn unusable_request_timeout() {
    let address: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let arguments = r#""video_id":"vid-1","prompt":"p","custom_llm_provider":"openai""#;
    Python::initialize();
    Python::attach(|py| {
        let request_payload = payload(&address, arguments);
        let settings = r#"{"environment":{"REQUEST_TIMEOUT":"-1"}}"#;
        let session = Py::new(py, NativeVideoEditSession::new(settings).unwrap()).unwrap();
        let error = session
            .call_method1(py, "edit", (request_payload, Vec::<Vec<u8>>::new()))
            .unwrap_err();
        assert!(error.is_instance_of::<PyRuntimeError>(py));
        let message = error.value(py).to_string();
        assert!(message.contains("REQUEST_TIMEOUT"), "{message}");
    });
}

#[test]
fn malformed_settings() {
    Python::initialize();
    Python::attach(|py| {
        let error = NativeVideoEditSession::new("not json").err().unwrap();
        assert!(error.is_instance_of::<PyValueError>(py));
        assert!(error.to_string().contains("not JSON"));
    });
}

#[test]
fn malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        let session = new_session(py);
        let error = session
            .call_method1(py, "create_character", ("not json", Vec::<Vec<u8>>::new()))
            .unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py));
        assert!(error.to_string().contains("not JSON"));
    });
}

#[test]
fn keyword_name_predicate() {
    assert!(is_litellm_owned_kwarg("client"));
    assert!(is_litellm_owned_kwarg("shared_session"));
    assert!(!is_litellm_owned_kwarg("prompt"));
    assert!(!is_litellm_owned_kwarg("model"));
}
