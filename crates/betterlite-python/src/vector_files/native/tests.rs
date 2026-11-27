#![cfg(test)]

use std::ffi::CStr;
use std::net::SocketAddr;
use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use rstest::rstest;

use super::NativeVectorFileSession;
use crate::vector_files::register;

const FILE: &str = r#"{"id":"file_1","object":"vector_store.file","vector_store_id":"vs_a","status":"completed","usage_bytes":12,"attributes":{"source":"doc.md"}}"#;
const PAGE: &str = r#"{"object":"list","data":[{"id":"file_1","object":"vector_store.file"}],"first_id":"file_1","last_id":"file_1","has_more":false}"#;
const CONTENT: &str =
    r#"{"object":"vector_store.file.content","data":[{"type":"text","text":"hello"}]}"#;
const UNAUTHORIZED: &str =
    r#"{"error":{"message":"bad key","type":"invalid_request_error","code":"invalid_api_key"}}"#;
const INVOKE: &CStr = c"import asyncio, inspect
async def invoke():
    result = getattr(session, method)(payload, blobs)
    if inspect.isawaitable(result):
        result = await result
    return result
answer = asyncio.run(invoke())
";

fn payload(address: &SocketAddr, method: &str) -> String {
    let arguments = match method {
        "create" | "acreate" => {
            r#""file_id":"file_1","attributes":{"source":"doc.md"},"extra_body":{"flag":true},"#
        }
        "list" | "alist" => r#""filter":"completed","limit":3,"order":"desc","#,
        "retrieve" | "aretrieve" | "retrieve_content" | "aretrieve_content" | "delete"
        | "adelete" => r#""file_id":"file_1","#,
        "update" | "aupdate" => {
            r#""file_id":"file_1","attributes":{"source":"new.md"},"extra_body":{"flag":true},"#
        }
        _ => panic!("unknown test operation"),
    };
    format!(
        r#"{{"vector_store_id":"vs_a",{arguments}"custom_llm_provider":"openai","kwargs":{{"api_base":"http://{address}/v1","api_key":"sk-test"}}}}"#
    )
}

fn invoke(settings: &str, method: &str, payload: &str) -> String {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "vector_file_test").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeVectorFileSession").unwrap());
        assert!(module.hasattr("NativeVectorFileError").unwrap());
        let session = module
            .getattr("NativeVectorFileSession")
            .unwrap()
            .call1((settings,))
            .unwrap();
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("method", method).unwrap();
        globals.set_item("payload", payload).unwrap();
        globals.set_item("blobs", Vec::<Vec<u8>>::new()).unwrap();
        py.run(INVOKE, Some(&globals), None).unwrap();
        let answer = globals.get_item("answer").unwrap().unwrap();
        answer.extract().unwrap()
    })
}

fn roundtrip(method: &str, response: &str, path: &str, sent: &[&str], answer: &str) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], response),
        Duration::ZERO,
    )
    .unwrap();
    let payload = payload(&serving.address, method);
    let text = invoke(r#"{"environment":{}}"#, method, &payload);
    let request = serving.finish().unwrap();
    assert!(request.line.starts_with(path), "{}", request.line);
    assert!(
        request
            .head
            .to_lowercase()
            .contains("authorization: bearer sk-test"),
        "{}",
        request.head
    );
    for fragment in sent {
        assert!(request.body.contains(fragment), "{}", request.body);
    }
    assert!(text.contains(answer), "{text}");
}

#[test]
fn register_adds_the_vector_file_class_and_error() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeVectorFileSession").unwrap());
        assert!(module.hasattr("NativeVectorFileError").unwrap());
    });
}

#[test]
fn a_malformed_session_text_is_an_error_not_a_panic() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(NativeVectorFileSession::new("not a session").is_err());
    });
}

#[rstest]
#[case::create("create")]
#[case::list("list")]
#[case::retrieve("retrieve")]
#[case::retrieve_content("retrieve_content")]
#[case::update("update")]
#[case::delete("delete")]
#[case::acreate("acreate")]
#[case::alist("alist")]
#[case::aretrieve("aretrieve")]
#[case::aretrieve_content("aretrieve_content")]
#[case::aupdate("aupdate")]
#[case::adelete("adelete")]
fn a_malformed_payload_is_a_value_error(#[case] method: &str) {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "vector_file_payload_test").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeVectorFileSession").unwrap();
        let session = class.call1((r#"{"environment":{}}"#,)).unwrap();
        let error = session
            .call_method1(method, ("not a call", Vec::<Vec<u8>>::new()))
            .unwrap_err();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn create_roundtrip() {
    roundtrip(
        "create",
        FILE,
        "POST /v1/vector_stores/vs_a/files HTTP/",
        &[
            r#""file_id":"file_1""#,
            r#""source":"doc.md""#,
            r#""flag":true"#,
        ],
        r#""id":"file_1""#,
    );
    roundtrip(
        "acreate",
        FILE,
        "POST /v1/vector_stores/vs_a/files HTTP/",
        &[
            r#""file_id":"file_1""#,
            r#""source":"doc.md""#,
            r#""flag":true"#,
        ],
        r#""id":"file_1""#,
    );
}

#[test]
fn list_roundtrip() {
    roundtrip(
        "list",
        PAGE,
        "GET /v1/vector_stores/vs_a/files?",
        &[],
        r#""has_more":false"#,
    );
    roundtrip(
        "alist",
        PAGE,
        "GET /v1/vector_stores/vs_a/files?",
        &[],
        r#""has_more":false"#,
    );
}

#[test]
fn retrieve_roundtrip() {
    roundtrip(
        "retrieve",
        FILE,
        "GET /v1/vector_stores/vs_a/files/file_1 HTTP/",
        &[],
        r#""id":"file_1""#,
    );
    roundtrip(
        "aretrieve",
        FILE,
        "GET /v1/vector_stores/vs_a/files/file_1 HTTP/",
        &[],
        r#""id":"file_1""#,
    );
}

#[test]
fn retrieve_content_roundtrip() {
    roundtrip(
        "retrieve_content",
        CONTENT,
        "GET /v1/vector_stores/vs_a/files/file_1/content HTTP/",
        &[],
        r#""object":"vector_store.file.content""#,
    );
    roundtrip(
        "aretrieve_content",
        CONTENT,
        "GET /v1/vector_stores/vs_a/files/file_1/content HTTP/",
        &[],
        r#""object":"vector_store.file.content""#,
    );
}

#[test]
fn update_roundtrip() {
    roundtrip(
        "update",
        FILE,
        "POST /v1/vector_stores/vs_a/files/file_1 HTTP/",
        &[r#""source":"new.md""#, r#""flag":true"#],
        r#""id":"file_1""#,
    );
    roundtrip(
        "aupdate",
        FILE,
        "POST /v1/vector_stores/vs_a/files/file_1 HTTP/",
        &[r#""source":"new.md""#, r#""flag":true"#],
        r#""id":"file_1""#,
    );
}

#[test]
fn delete_roundtrip() {
    roundtrip(
        "delete",
        FILE,
        "DELETE /v1/vector_stores/vs_a/files/file_1",
        &[],
        r#""id":"file_1""#,
    );
    roundtrip(
        "adelete",
        FILE,
        "DELETE /v1/vector_stores/vs_a/files/file_1",
        &[],
        r#""id":"file_1""#,
    );
}

#[test]
fn a_provider_error_keeps_its_class_and_cause() {
    Python::initialize();
    let serving = serve(
        reply(
            "401 Unauthorized",
            &[("content-type", "application/json")],
            UNAUTHORIZED,
        ),
        Duration::ZERO,
    )
    .unwrap();
    let text = payload(&serving.address, "create");
    Python::attach(|py| {
        let module = PyModule::new(py, "vector_file_error_test").unwrap();
        register(&module).unwrap();
        let session = module
            .getattr("NativeVectorFileSession")
            .unwrap()
            .call1((r#"{"environment":{}}"#,))
            .unwrap();
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("method", "create").unwrap();
        globals.set_item("payload", text).unwrap();
        globals.set_item("blobs", Vec::<Vec<u8>>::new()).unwrap();
        let error = py.run(INVOKE, Some(&globals), None).unwrap_err();
        let class = module.getattr("NativeVectorFileError").unwrap();
        assert!(error.get_type(py).is(&class));
        assert!(
            error
                .to_string()
                .contains(r#""type":"AuthenticationError""#),
            "{error}"
        );
    });
    let _request = serving.finish().unwrap();
}

#[test]
fn a_call_that_outlives_its_timeout_raises_the_family_error() {
    Python::initialize();
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], FILE),
        Duration::from_millis(500),
    )
    .unwrap();
    let text = payload(&serving.address, "create").replace(
        r#""file_id":"file_1","#,
        r#""file_id":"file_1","timeout":0.05,"#,
    );
    Python::attach(|py| {
        let module = PyModule::new(py, "vector_file_timeout_test").unwrap();
        register(&module).unwrap();
        let session = module
            .getattr("NativeVectorFileSession")
            .unwrap()
            .call1((r#"{"environment":{}}"#,))
            .unwrap();
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("method", "create").unwrap();
        globals.set_item("payload", text).unwrap();
        globals.set_item("blobs", Vec::<Vec<u8>>::new()).unwrap();
        let error = py.run(INVOKE, Some(&globals), None).unwrap_err();
        let class = module.getattr("NativeVectorFileError").unwrap();
        assert!(error.get_type(py).is(&class));
        assert!(error.to_string().contains(r#""type":"Timeout""#), "{error}");
    });
    let _request = serving.finish().unwrap();
}

#[test]
fn session_settings_supply_the_credentials_the_call_uses() {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], FILE),
        Duration::ZERO,
    )
    .unwrap();
    let settings = format!(
        r#"{{"environment":{{}},"api_base":"http://{}/v1","api_key":"sk-global"}}"#,
        serving.address
    );
    let text = invoke(
        &settings,
        "create",
        r#"{"vector_store_id":"vs_a","file_id":"file_1","kwargs":{}}"#,
    );
    assert!(text.contains(r#""id":"file_1""#), "{text}");
    let request = serving.finish().unwrap();
    assert!(
        request
            .head
            .to_lowercase()
            .contains("authorization: bearer sk-global"),
        "{}",
        request.head
    );
    assert!(
        request
            .line
            .starts_with("POST /v1/vector_stores/vs_a/files HTTP/"),
        "{}",
        request.line
    );
}
