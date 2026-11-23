#![cfg(test)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::ffi::CStr;

use super::super::register;
use super::NativeEmbeddingSession;

const SESSION: &str = r#"{"environment": {}}"#;
const MOCK: &str =
    r#"{"arguments": {"model": "text-embedding-3-small", "input": "hi", "mock_response": [0.5]}}"#;
const AWAIT_MOCK: &CStr = c"import asyncio
async def main():
    return await session.aembedding(request, [])
answer = asyncio.run(main())
";

#[test]
fn register_adds_the_embedding_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeEmbeddingSession").unwrap());
        assert!(module.hasattr("NativeEmbeddingError").unwrap());
    });
}

#[test]
fn a_blocking_mock_call_answers_json_without_the_provider() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEmbeddingSession::new(SESSION).unwrap();
        let text = session.embedding(py, MOCK, &PyList::empty(py)).unwrap();
        assert!(text.contains(r#""object":"list""#));
        assert!(text.contains(r#""embedding":[0.5]"#));
        assert!(!text.contains("custom_llm_provider"));
    });
}

#[test]
fn an_awaited_mock_call_answers_json_naming_the_provider() {
    Python::initialize();
    Python::attach(|py| {
        let session = Bound::new(py, NativeEmbeddingSession::new(SESSION).unwrap()).unwrap();
        let globals = PyDict::new(py);
        globals.set_item("session", session).unwrap();
        globals.set_item("request", MOCK).unwrap();
        py.run(AWAIT_MOCK, Some(&globals), None).unwrap();
        let answer = globals.get_item("answer").unwrap().unwrap();
        let text: String = answer.extract().unwrap();
        assert!(text.contains(r#""embedding":[0.5]"#));
        assert!(text.contains(r#""custom_llm_provider":"openai""#));
    });
}

#[test]
fn a_malformed_request_text_is_a_value_error_on_both_entries() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEmbeddingSession::new(SESSION).unwrap();
        let blocking = session
            .embedding(py, "not json", &PyList::empty(py))
            .unwrap_err();
        assert!(blocking.is_instance_of::<PyValueError>(py));
        let awaited = session
            .aembedding(py, "not json", &PyList::empty(py))
            .unwrap_err();
        assert!(awaited.is_instance_of::<PyValueError>(py));
    });
}
