#![cfg(test)]

use betterlite_core::api::vector_store_http_create_read::error::CreateReadError;
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, WriteError,
};
use betterlite_core::api::vector_store_read::error::{
    VectorStoreReadError, failure as read_failure,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use pyo3::prelude::*;

use super::super::error::{NativeFailure, raise};
use super::super::register;
use super::NativeVectorManageSession;

#[test]
fn register_adds_the_vector_manage_class_and_error() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").expect("the probe module builds");
        register(&module).expect("the family registers");
        assert!(
            module
                .hasattr("NativeVectorManageSession")
                .expect("the module answers")
        );
        assert!(
            module
                .hasattr("NativeVectorManageError")
                .expect("the module answers")
        );
    });
}

#[test]
fn a_malformed_session_text_is_an_error_not_a_panic() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            NativeVectorManageSession::new("not a session"),
            Err(_refused)
        ));
    });
}

#[test]
fn a_raised_read_failure_is_the_registered_error_class() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").expect("the probe module builds");
        register(&module).expect("the family registers");
        let redactor =
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
        let failure = NativeFailure::Read {
            failure: read_failure(
                VectorStoreReadError::Read(CreateReadError::Write(WriteError::Provider(
                    BaseLlmException {
                        status_code: 401,
                        message: String::from("missing"),
                        headers: HeaderMap::new(),
                        status_code_is_synthesized: false,
                    },
                ))),
                "openai",
                &redactor,
            ),
            provider: String::from("openai"),
        };
        let error = raise(py, &failure);
        let class = module
            .getattr("NativeVectorManageError")
            .expect("the class registered");
        assert!(error.get_type(py).is(&class));
    });
}
