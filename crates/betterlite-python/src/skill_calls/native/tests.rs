#![cfg(test)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::NativeSkillSession;
use crate::skill_calls::error::NO_DATABASE;

fn hosted_error(answer: PyResult<String>) {
    let error = answer.err().unwrap();
    let text = error.to_string();
    assert!(text.contains("APIConnectionError"), "{text}");
    assert!(text.contains(NO_DATABASE), "{text}");
}

#[test]
fn the_native_session_answers_a_hosted_get_call_with_the_connection_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeSkillSession::new(r#"{"environment":{}}"#).unwrap();
        hosted_error(session.get_skill(
            py,
            r#"{"skill_id":"s","custom_llm_provider":"litellm_proxy"}"#,
            Vec::new(),
        ));
    });
}

#[test]
fn the_native_session_answers_a_hosted_create_call_with_the_connection_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeSkillSession::new(r#"{"environment":{}}"#).unwrap();
        hosted_error(session.create_skill(
            py,
            r#"{"display_title":"T","custom_llm_provider":"litellm_proxy"}"#,
            Vec::new(),
        ));
    });
}

#[test]
fn the_native_session_answers_a_hosted_list_call_with_the_connection_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeSkillSession::new(r#"{"environment":{}}"#).unwrap();
        hosted_error(session.list_skills(
            py,
            r#"{"source":"custom","custom_llm_provider":"litellm_proxy"}"#,
            Vec::new(),
        ));
    });
}

#[test]
fn the_native_session_answers_a_hosted_delete_call_with_the_connection_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeSkillSession::new(r#"{"environment":{}}"#).unwrap();
        hosted_error(session.delete_skill(
            py,
            r#"{"skill_id":"s","custom_llm_provider":"litellm_proxy"}"#,
            Vec::new(),
        ));
    });
}

#[test]
fn a_request_that_is_not_a_json_object_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeSkillSession::new(r#"{"environment":{}}"#).unwrap();
        let error = session.get_skill(py, "[1, 2]", Vec::new()).err().unwrap();
        assert!(error.is_instance_of::<PyValueError>(py), "{error}");
    });
}

#[test]
fn settings_that_are_not_json_are_a_value_error() {
    Python::initialize();
    Python::attach(|_py| {
        let error = NativeSkillSession::new("{").err().unwrap();
        assert!(
            error
                .to_string()
                .contains("the settings are not a JSON object"),
            "{error}"
        );
    });
}
