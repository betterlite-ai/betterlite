#![cfg(test)]

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyModule;
use pyo3::{PyErr, exceptions::PyValueError};

use super::convert::{ConvertError, NativeDecisionPayload, parse};
use super::core::SessionCore;
use super::error::{SessionError, SessionFailure};
use super::{NativeDecisionSession, register, supported_decisions_providers};
use crate::completion;

#[test]
fn native_exports_every_litellm_decision_provider() {
    assert_eq!(
        supported_decisions_providers(),
        vec![
            "openai",
            "openrouter",
            "azure_ai",
            "perplexity",
            "typesafe",
            "strands_decider",
            "cloudflare",
            "hosted_vllm",
        ]
    );
}

#[test]
fn register_adds_the_decision_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        completion::register(&module).unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeDecisionSession").unwrap());
        assert!(module.hasattr("NativeDecisionError").unwrap());
        assert!(module.hasattr("supported_decisions_providers").unwrap());
    });
}

#[test]
fn native_session_answers_a_sandbox_tool_registration_with_null() {
    Python::initialize();
    let answer = Python::attach(|py| {
        let session = NativeDecisionSession {
            core: Arc::new(SessionCore::build(r#"{"environment":{}}"#).unwrap()),
            error_type: PyErr::new_type(py, c"probe.NativeDecisionError", None, None, None)
                .unwrap(),
        };
        session.call(
            py,
            r#"{"operation": "register_tools", "arguments": {"tools": [], "environment": {}}}"#,
            &py.None().into_bound(py).into_any(),
        )
    })
    .unwrap();
    assert_eq!(answer, "null");
}

#[test]
fn native_async_session_answers_a_sandbox_tool_registration_with_null() {
    Python::initialize();
    let answer = Python::attach(|py| -> PyResult<String> {
        let session = NativeDecisionSession {
            core: Arc::new(SessionCore::build(r#"{"environment":{}}"#)?),
            error_type: PyErr::new_type(py, c"probe.NativeDecisionError", None, None, None)?,
        };
        let blobs = py.None().into_bound(py).into_any();
        let awaitable = session.acall(
            py,
            r#"{"operation": "register_tools", "arguments": {"tools": [], "environment": {}}}"#,
            &blobs,
        )?;
        py.import("asyncio")?
            .call_method1("run", (awaitable,))?
            .extract()
    })
    .unwrap();
    assert_eq!(answer, "null");
}

#[test]
fn native_session_maps_conversion_errors_to_python_errors() {
    Python::initialize();
    Python::attach(|py| {
        let malformed_settings = NativeDecisionSession::new(py, "not json");
        assert!(matches!(
            malformed_settings,
            Err(error) if error.is_instance_of::<PyValueError>(py)
        ));
        let session = NativeDecisionSession {
            core: Arc::new(SessionCore::build(r#"{"environment":{}}"#).unwrap()),
            error_type: PyErr::new_type(py, c"probe.NativeDecisionError", None, None, None)
                .unwrap(),
        };
        let blobs = py.None().into_bound(py).into_any();
        let malformed = session.call(py, "not json", &blobs).unwrap_err();
        assert!(malformed.is_instance_of::<PyValueError>(py));
        let unknown = session
            .call(py, r#"{"operation":"not_a_call"}"#, &blobs)
            .unwrap_err();
        assert!(unknown.to_string().contains("not_a_call operation"));

        let source = parse::<super::convert::CallWire>("not json").unwrap_err();
        let setup = SessionFailure::from(SessionError::Json(source));
        assert!(matches!(setup, SessionFailure::Setup(_)));
        let invalid = SessionFailure::from(ConvertError::Container);
        assert!(matches!(
            invalid,
            SessionFailure::Invalid(ConvertError::Container)
        ));
    });
}

#[test]
fn native_session_raises_the_registered_error_with_its_payload() {
    Python::initialize();
    Python::attach(|py| {
        let error_type = PyErr::new_type(py, c"probe.NativeDecisionError", None, None, None)
            .unwrap()
            .unbind();
        let failure = SessionFailure::Raised(Box::new(NativeDecisionPayload::builtin(
            "ValueError",
            String::from("invalid sandbox call"),
        )));
        let error = failure.into_pyerr(py, &error_type);
        assert!(error.to_string().contains("invalid sandbox call"));
    });
}
