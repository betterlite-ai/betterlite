#![cfg(test)]

use std::net::TcpListener;

use indexmap::IndexMap;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;

use betterlite_core::api::skills::get_delete::error::SkillAction;
use betterlite_error::error::class_message;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_wrapper::error::class_name;

use super::error::{NO_DATABASE, SkillFailure};
use super::native::NativeSkillSession;
use super::session::{Mode, SkillCore};

fn arguments(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn core() -> SkillCore {
    SkillCore::build(r#"{"environment":{}}"#).ok().unwrap()
}

// A port nothing listens on any more: the connect is refused at once and the call reaches the transport.
fn closed_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn refusal(failure: &SkillFailure) -> String {
    assert_eq!(class_name(&failure.class), "APIConnectionError");
    let message = class_message(&failure.class);
    assert!(
        message.contains("the connection could not be made"),
        "{message}"
    );
    assert!(!message.contains(NO_DATABASE), "{message}");
    message
}

#[test]
fn a_get_call_for_an_http_provider_reaches_the_transport() {
    let base = closed_base();
    let request = format!(r#"{{"skill_id": "s", "api_key": "sk-test", "api_base": "{base}"}}"#);
    let failure = get_runtime()
        .block_on(core().skill(SkillAction::Get, arguments(&request)))
        .err()
        .unwrap();
    refusal(&failure);
}

#[test]
fn a_create_call_for_an_http_provider_reaches_the_transport() {
    let base = closed_base();
    let request =
        format!(r#"{{"display_title": "T", "api_key": "sk-test", "api_base": "{base}"}}"#);
    let failure = get_runtime()
        .block_on(core().execute(Mode::Create, (arguments(&request), Vec::new())))
        .err()
        .unwrap();
    refusal(&failure);
}

#[test]
fn a_list_call_for_an_http_provider_reaches_the_transport() {
    let base = closed_base();
    let request = format!(r#"{{"source": "custom", "api_key": "sk-test", "api_base": "{base}"}}"#);
    let failure = get_runtime()
        .block_on(core().execute(Mode::List, (arguments(&request), Vec::new())))
        .err()
        .unwrap();
    refusal(&failure);
}

#[test]
fn a_hosted_call_answers_without_a_database() {
    let request = r#"{"skill_id": "s", "custom_llm_provider": "litellm_proxy"}"#;
    let failure = get_runtime()
        .block_on(core().skill(SkillAction::Get, arguments(request)))
        .err()
        .unwrap();
    assert_eq!(class_name(&failure.class), "APIConnectionError");
    assert!(class_message(&failure.class).contains(NO_DATABASE));
}

#[test]
fn register_adds_the_skill_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        super::register(&module).unwrap();
        assert!(module.hasattr("NativeSkillSession").unwrap());
        assert!(module.hasattr("NativeSkillError").unwrap());
    });
}

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
