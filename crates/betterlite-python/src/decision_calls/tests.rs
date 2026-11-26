#![cfg(test)]

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyModule;

use super::core::SessionCore;
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
