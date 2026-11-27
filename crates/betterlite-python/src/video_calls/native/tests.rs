#![cfg(test)]

use std::ffi::CStr;
use std::net::SocketAddr;
use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use pyo3::exceptions::PyValueError;
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _, PyModule};
use pyo3::{Py, Python};

use super::NativeVideoSession;
use crate::video_calls::error::NativeVideoError;
use crate::video_calls::register;

const VIDEO: &str = r#"{"id":"video_test","object":"video","status":"completed","progress":42,"model":"sora-2","seconds":"8"}"#;
const PAGE: &str = r#"{"object":"list","data":[{"id":"video_test","object":"video","status":"completed"}],"has_more":false}"#;
const INVOKE: &CStr = c"import asyncio, inspect
async def invoke():
    result = getattr(session, method)(payload, blobs)
    if inspect.isawaitable(result):
        result = await result
    return result
answer = asyncio.run(invoke())
";

fn invoke(method: &str, payload: &str) -> String {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "video_test").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeVideoError").unwrap());
        let session = module
            .getattr("NativeVideoSession")
            .unwrap()
            .call1((r#"{"environment":{}}"#,))
            .unwrap();
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("method", method).unwrap();
        globals.set_item("payload", payload).unwrap();
        globals.set_item("blobs", Vec::<Vec<u8>>::new()).unwrap();
        py.run(INVOKE, Some(&globals), None).unwrap();
        let answer = globals.get_item("answer").unwrap().unwrap();
        if method.ends_with("content") {
            assert_eq!(answer.extract::<Vec<u8>>().unwrap(), b"video-bytes");
            return String::from("video-bytes");
        }
        answer.extract().unwrap()
    })
}

fn payload(address: &SocketAddr, method: &str) -> String {
    let arguments = match method {
        "generation" | "ageneration" => r#""prompt":"a kite","model":"sora-2","#,
        "remix" | "aremix" => r#""prompt":"a kite","video_id":"video_test","#,
        "content" | "acontent" | "status" | "astatus" => r#""video_id":"video_test","#,
        "list" | "alist" => "",
        _ => panic!("unknown test operation"),
    };
    format!(
        r#"{{"arguments":{{{arguments}"custom_llm_provider":"openai","api_base":"http://{address}/v1","api_key":"sk-test"}}}}"#
    )
}

fn roundtrip(method: &str, response: &str, path: &str) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], response),
        Duration::ZERO,
    )
    .unwrap();
    let payload = payload(&serving.address, method);
    let text = invoke(method, &payload);
    let request = serving.finish().unwrap();
    assert!(request.line.starts_with(path), "{}", request.line);
    assert!(
        request
            .head
            .to_lowercase()
            .contains("authorization: bearer sk-test")
    );
    if method.ends_with("content") {
        assert_eq!(text, "video-bytes");
    } else if method.ends_with("list") {
        assert!(text.contains(r#""has_more":false"#), "{text}");
        assert!(text.contains(r#""status":"completed""#), "{text}");
    } else {
        assert!(text.contains(r#""progress":42"#), "{text}");
        assert!(text.contains(r#""status":"completed""#), "{text}");
    }
}

#[test]
fn generation_roundtrip() {
    roundtrip("generation", VIDEO, "POST /v1/videos");
    roundtrip("ageneration", VIDEO, "POST /v1/videos");
}

#[test]
fn content_roundtrip() {
    roundtrip(
        "content",
        "video-bytes",
        "GET /v1/videos/video_test/content",
    );
    roundtrip(
        "acontent",
        "video-bytes",
        "GET /v1/videos/video_test/content",
    );
}

#[test]
fn status_roundtrip() {
    roundtrip("status", VIDEO, "GET /v1/videos/video_test");
    roundtrip("astatus", VIDEO, "GET /v1/videos/video_test");
}

#[test]
fn remix_roundtrip() {
    roundtrip("remix", VIDEO, "POST /v1/videos/video_test/remix");
    roundtrip("aremix", VIDEO, "POST /v1/videos/video_test/remix");
}

#[test]
fn list_roundtrip() {
    roundtrip("list", PAGE, "GET /v1/videos");
    roundtrip("alist", PAGE, "GET /v1/videos");
}

#[test]
fn malformed_settings() {
    Python::initialize();
    Python::attach(|py| {
        let error = NativeVideoSession::new("not json").err().unwrap();
        assert!(error.is_instance_of::<PyValueError>(py));
        assert!(error.to_string().contains("not JSON"));
    });
}

fn failed_call(method: &str) {
    let serving = serve(
        reply(
            "401 Unauthorized",
            &[("content-type", "application/json")],
            r#"{"error":{"message":"Incorrect API key provided: bad-key","type":"invalid_request_error"}}"#,
        ),
        Duration::ZERO,
    )
    .unwrap();
    let payload = payload(&serving.address, method);
    Python::initialize();
    Python::attach(|py| {
        let session = Py::new(
            py,
            NativeVideoSession::new(r#"{"environment":{}}"#).unwrap(),
        )
        .unwrap();
        let error = session
            .call_method1(py, method, (payload, Vec::<Vec<u8>>::new()))
            .unwrap_err();
        assert!(error.is_instance_of::<NativeVideoError>(py));
        let message = error.value(py).to_string();
        assert!(
            message.contains(r#""type":"AuthenticationError""#),
            "{message}"
        );
        assert!(message.contains(r#""status_code":401"#), "{message}");
        assert!(message.contains("Incorrect API key provided"), "{message}");
    });
    serving.finish().unwrap();
}

#[test]
fn generation_error() {
    failed_call("generation");
}

#[test]
fn content_error() {
    failed_call("content");
}

#[test]
fn status_error() {
    failed_call("status");
}

#[test]
fn remix_error() {
    failed_call("remix");
}

#[test]
fn mock_before_resolution() {
    let payload = r#"{"arguments":{"prompt":"p","model":"unresolvable-model","mock_response":{"id":"mock-id","object":"video","status":"queued"}}}"#;
    let text = invoke("generation", payload);
    assert!(text.contains(r#""id":"mock-id""#));
    assert!(text.contains(r#""status":"queued""#));
}
