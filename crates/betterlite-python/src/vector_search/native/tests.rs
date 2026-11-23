#![cfg(test)]

use betterlite_core::api::vector_store_search::error::VectorStoreSearchError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use pyo3::prelude::*;

use super::super::error::raise;
use super::super::register;
use super::NativeVectorSearchSession;

#[test]
fn register_adds_the_vector_search_class_and_error() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeVectorSearchSession").unwrap());
        assert!(module.hasattr("NativeVectorSearchError").unwrap());
    });
}

#[test]
fn a_malformed_session_text_is_an_error_not_a_panic() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            NativeVectorSearchSession::new("not a session"),
            Err(_refused)
        ));
    });
}

#[test]
fn a_session_takes_its_globals_and_an_environment() {
    Python::initialize();
    Python::attach(|_py| {
        let settings =
            r#"{"environment": {}, "api_key": "sk-test", "api_base": "http://127.0.0.1:9/v1"}"#;
        assert!(matches!(
            NativeVectorSearchSession::new(settings),
            Ok(_session)
        ));
    });
}

#[test]
fn a_raised_failure_is_the_registered_error_class() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
        let failure = VectorStoreSearchError::Unsupported {
            operation: "search",
            provider: String::from("anthropic"),
        };
        let error = raise(py, &failure, &redactor);
        let class = module.getattr("NativeVectorSearchError").unwrap();
        assert!(error.get_type(py).is(&class));
    });
}

#[test]
fn a_refused_executor_is_a_plain_type_error() {
    Python::initialize();
    Python::attach(|py| {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
        let error = raise(
            py,
            &VectorStoreSearchError::InvalidEmbeddingExecutor,
            &redactor,
        );
        let class = py.get_type::<pyo3::exceptions::PyTypeError>();
        assert!(error.get_type(py).is(&class));
    });
}
