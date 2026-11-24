#![cfg(test)]

use pyo3::prelude::*;

use super::super::register;
use super::NativeGenaiSession;
use crate::genai::convert::ConvertError;

const SESSION: &str = r#"{"environment": {}}"#;

#[test]
fn register_adds_the_genai_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeGenaiSession").unwrap());
        assert!(module.hasattr("NativeGenaiStream").unwrap());
        assert!(module.hasattr("NativeGenaiError").unwrap());
    });
}

#[test]
fn a_malformed_session_text_is_an_error_not_a_panic() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(NativeGenaiSession::new("not a session").is_err());
    });
}

#[test]
fn a_file_marker_is_refused_before_any_request() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeGenaiSession::new(SESSION).unwrap();
        let payload = r#"{"model": "gemini/gemini-2.5-flash", "contents": [{"parts": [{"$file": {"blob": 0, "name": "a.txt"}}]}], "kwargs": {}}"#;
        let message = session
            .content(py, payload, vec![b"hi".to_vec()])
            .err()
            .map(|error| error.to_string());
        let refusal = ConvertError::Files.to_string();
        assert_eq!(message, Some(format!("ValueError: {refusal}")));
    });
}
