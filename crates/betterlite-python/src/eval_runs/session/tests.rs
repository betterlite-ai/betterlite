#![cfg(test)]

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_core::api::eval_lifecycle::EvalLifecycleConfigLookup as _;
use betterlite_core::api::eval_run_create_list::providers::EvalRunProviders as _;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_models::decisions::Json;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::BaseEvalsAPIConfig as _;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use secrecy::SecretString;

use super::EvalCore;
use super::NativeEvalRunSession;
use crate::eval_runs::configs::{EvalConfigs, ModuleSettings};

const ENVIRONMENT: &str = r#"{"environment":{}}"#;
const CREATED: &str =
    r#"{"id":"run_created","created_at":1,"status":"queued","data_source":{},"eval_id":"eval_1"}"#;
const FETCHED: &str =
    r#"{"id":"run_fetched","created_at":1,"status":"queued","data_source":{},"eval_id":"eval_1"}"#;
const LISTED: &str = r#"{"object":"list","data":[],"has_more":false}"#;
const CANCELLED: &str = r#"{"id":"run_1","status":"cancelled"}"#;
const DELETED: &str = r#"{"run_id":"run_1","deleted":true}"#;

const JSON_HEADERS: &[(&str, &str)] = &[("content-type", "application/json")];

// one fixture server per request: it answers with the body it was given and hands back the request line it saw
fn serve_answer(body: &str) -> Serving {
    serve(reply("200 OK", JSON_HEADERS, body), Duration::ZERO).expect("the fixture serves")
}

fn base(serving: &Serving) -> String {
    format!("http://{}", serving.address)
}

// a port nothing listens on any more: the connect is refused at once and reaches the transport
fn closed_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port is found");
    let port = listener
        .local_addr()
        .expect("the free port has an address")
        .port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn call(base: &str, fields: &str) -> String {
    format!(r#"{{{fields}"api_base":"{base}","api_key":"sk-test"}}"#)
}

fn create_call(base: &str) -> String {
    format!(
        r#"{{"eval_id":"eval_1","data_source":{{"type":"custom"}},"api_base":"{base}","api_key":"sk-test"}}"#
    )
}

fn list_call(base: &str) -> String {
    call(base, r#""eval_id":"eval_1","limit":5,"#)
}

fn run_call(base: &str) -> String {
    call(base, r#""eval_id":"eval_1","run_id":"run_1","#)
}

fn message(py: Python<'_>, error: &PyErr) -> String {
    error.value(py).to_string()
}

// the fixture answered this call with the body whose marker the reply carries, and saw the request line the operation builds
fn assert_reply_and_request(serving: Serving, reply: &str, marker: &str, request_line: &str) {
    assert!(reply.contains(marker), "{reply}");
    let request = serving.finish().expect("the fixture answers once");
    assert!(request.line.starts_with(request_line), "{}", request.line);
}

#[test]
fn register_adds_the_run_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").expect("the probe module exists");
        super::super::register(&module).expect("register succeeds");
        assert!(
            module
                .hasattr("NativeEvalRunSession")
                .expect("the module reads")
        );
        assert!(
            module
                .hasattr("NativeEvalRunError")
                .expect("the module reads")
        );
    });
}

#[test]
fn session_bad_settings_text_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let Err(error) = NativeEvalRunSession::new("not json") else {
            panic!("settings must parse");
        };
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn session_bad_json_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEvalRunSession::new(ENVIRONMENT).expect("the session builds");
        let error = session
            .create_run(py, "not json", Vec::new())
            .expect_err("a bad payload must fail");
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn session_sync_calls_reach_create_list_and_get() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEvalRunSession::new(ENVIRONMENT).expect("the session builds");
        let created = serve_answer(CREATED);
        let created_reply = session
            .create_run(py, &create_call(&base(&created)), Vec::new())
            .expect("create answers");
        assert_reply_and_request(
            created,
            &created_reply,
            "run_created",
            "POST /v1/evals/eval_1/runs HTTP/1.1",
        );
        let listed = serve_answer(LISTED);
        let listed_reply = session
            .list_runs(py, &list_call(&base(&listed)), Vec::new())
            .expect("list answers");
        assert_reply_and_request(
            listed,
            &listed_reply,
            r#""object":"list""#,
            "GET /v1/evals/eval_1/runs?limit=5 HTTP/1.1",
        );
        let fetched = serve_answer(FETCHED);
        let fetched_reply = session
            .get_run(py, &run_call(&base(&fetched)), Vec::new())
            .expect("get answers");
        assert_reply_and_request(
            fetched,
            &fetched_reply,
            "run_fetched",
            "GET /v1/evals/eval_1/runs/run_1 HTTP/1.1",
        );
    });
}

#[test]
fn session_sync_calls_reach_cancel_and_delete() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEvalRunSession::new(ENVIRONMENT).expect("the session builds");
        let cancelled = serve_answer(CANCELLED);
        let cancelled_reply = session
            .cancel_run(py, &run_call(&base(&cancelled)), Vec::new())
            .expect("cancel answers");
        assert_reply_and_request(
            cancelled,
            &cancelled_reply,
            r#""status":"cancelled""#,
            "POST /v1/evals/eval_1/runs/run_1/cancel HTTP/1.1",
        );
        let deleted = serve_answer(DELETED);
        let deleted_reply = session
            .delete_run(py, &run_call(&base(&deleted)), Vec::new())
            .expect("delete answers");
        assert_reply_and_request(
            deleted,
            &deleted_reply,
            r#""deleted":true"#,
            "DELETE /v1/evals/eval_1/runs/run_1 HTTP/1.1",
        );
    });
}

// the native async twins need the running loop the facade calls them from
const AWAIT_CALLS: &CStr = c"import asyncio
async def main():
    answers = []
    for method, request in calls:
        answers.append(await getattr(session, method)(request, []))
    return answers
answers = asyncio.run(main())
";

// every async twin answers on the loop the facade calls it from, one after another, in call order
fn await_calls(
    py: Python<'_>,
    session: NativeEvalRunSession,
    calls: Vec<(&str, String)>,
) -> Vec<String> {
    let globals = PyDict::new(py);
    globals
        .set_item("session", session)
        .expect("the globals take the session");
    globals
        .set_item("calls", calls)
        .expect("the globals take the calls");
    py.run(AWAIT_CALLS, Some(&globals), None)
        .expect("the awaits finish");
    globals
        .get_item("answers")
        .expect("the globals hold the answers")
        .expect("the answers were set")
        .extract()
        .expect("the answers are text")
}

#[test]
fn session_async_calls_reach_create_list_and_get() {
    Python::initialize();
    let created = serve_answer(CREATED);
    let listed = serve_answer(LISTED);
    let fetched = serve_answer(FETCHED);
    let calls = vec![
        ("acreate_run", create_call(&base(&created))),
        ("alist_runs", list_call(&base(&listed))),
        ("aget_run", run_call(&base(&fetched))),
    ];
    let answers = Python::attach(|py| {
        let session = NativeEvalRunSession::new(ENVIRONMENT).expect("the session builds");
        await_calls(py, session, calls)
    });
    assert_reply_and_request(
        created,
        &answers[0],
        "run_created",
        "POST /v1/evals/eval_1/runs HTTP/1.1",
    );
    assert_reply_and_request(
        listed,
        &answers[1],
        r#""object":"list""#,
        "GET /v1/evals/eval_1/runs?limit=5 HTTP/1.1",
    );
    assert_reply_and_request(
        fetched,
        &answers[2],
        "run_fetched",
        "GET /v1/evals/eval_1/runs/run_1 HTTP/1.1",
    );
}

#[test]
fn session_async_calls_reach_cancel_and_delete() {
    Python::initialize();
    let cancelled = serve_answer(CANCELLED);
    let deleted = serve_answer(DELETED);
    let calls = vec![
        ("acancel_run", run_call(&base(&cancelled))),
        ("adelete_run", run_call(&base(&deleted))),
    ];
    let answers = Python::attach(|py| {
        let session = NativeEvalRunSession::new(ENVIRONMENT).expect("the session builds");
        await_calls(py, session, calls)
    });
    assert_reply_and_request(
        cancelled,
        &answers[0],
        r#""status":"cancelled""#,
        "POST /v1/evals/eval_1/runs/run_1/cancel HTTP/1.1",
    );
    assert_reply_and_request(
        deleted,
        &answers[1],
        r#""deleted":true"#,
        "DELETE /v1/evals/eval_1/runs/run_1 HTTP/1.1",
    );
}

#[test]
fn session_a_wrong_typed_field_is_a_bad_request() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEvalRunSession::new(ENVIRONMENT).expect("the session builds");
        let error = session
            .list_runs(py, r#"{"eval_id": "eval_1", "limit": "many"}"#, Vec::new())
            .expect_err("a wrong-typed limit must fail");
        let text = message(py, &error);
        assert!(text.contains(r#""type":"BadRequestError""#), "{text}");
        assert!(text.contains(r#""status_code":400"#), "{text}");
        assert!(text.contains(r#""llm_provider":"openai""#), "{text}");
        assert!(text.contains(r#""param":"limit""#), "{text}");
        assert!(
            text.contains("the limit field does not have the declared type"),
            "{text}"
        );
    });
}

#[test]
fn session_a_refused_connection_is_a_synthesized_500() {
    Python::initialize();
    let base = closed_base();
    Python::attach(|py| {
        let session = NativeEvalRunSession::new(ENVIRONMENT).expect("the session builds");
        let error = session
            .get_run(py, &run_call(&base), Vec::new())
            .expect_err("a refused connection must fail");
        let text = message(py, &error);
        assert!(text.contains(r#""type":"InternalServerError""#), "{text}");
        assert!(text.contains(r#""status_code":500"#), "{text}");
        assert!(text.contains(r#""llm_provider":"openai""#), "{text}");
    });
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([("OPENAI_API_KEY".to_owned(), "env-key".to_owned())])
}

fn authorization(configs: &EvalConfigs) -> String {
    let config = configs
        .evals_config(EvalsConfigTag::OpenAI)
        .expect("openai has a config");
    let headers = config
        .validate_environment(Headers::new(), None)
        .expect("a key resolves");
    headers
        .get("Authorization")
        .cloned()
        .expect("the key is sent")
}

#[test]
fn configs_read_the_module_key_before_the_openai_key_and_the_environment() {
    let both = ModuleSettings {
        api_key: Some(SecretString::from("module-key")),
        openai_key: Some(SecretString::from("openai-key")),
    };
    assert_eq!(
        authorization(&EvalConfigs::new(both, &environment())),
        "Bearer module-key"
    );
    let fallback = ModuleSettings {
        api_key: None,
        openai_key: Some(SecretString::from("openai-key")),
    };
    assert_eq!(
        authorization(&EvalConfigs::new(fallback, &environment())),
        "Bearer openai-key"
    );
    let empty = ModuleSettings {
        api_key: Some(SecretString::from("")),
        openai_key: None,
    };
    assert_eq!(
        authorization(&EvalConfigs::new(empty, &environment())),
        "Bearer env-key"
    );
    let none = ModuleSettings::default();
    let configs = EvalConfigs::new(none, &BTreeMap::new());
    let config = configs
        .evals_config(EvalsConfigTag::OpenAI)
        .expect("openai has a config");
    assert!(matches!(
        config.validate_environment(Headers::new(), None),
        Err(EvalsError::ApiKeyRequired)
    ));
}

#[test]
fn configs_serve_only_openai_to_the_lifecycle_service() {
    let configs = EvalConfigs::new(ModuleSettings::default(), &environment());
    assert!(configs.eval_config("openai").is_some());
    assert!(configs.eval_config("anthropic").is_none());
    let config = configs.eval_config("openai").expect("openai has a config");
    let headers = config
        .validate_environment(Headers::new(), None)
        .expect("a key resolves");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer env-key")
    );
}

// LiteLLM keeps the caller's litellm_call_id on the logging object it builds for the call
#[test]
fn session_logging_keeps_the_callers_call_id() {
    let core = EvalCore::build(ENVIRONMENT).expect("the core builds");
    let kwargs = IndexMap::from([(
        "litellm_call_id".to_owned(),
        Json::Text("call-1".to_owned()),
    )]);
    let state =
        super::logging::state(&core, "create_run", &kwargs).expect("the logging state builds");
    assert_eq!(state.litellm_call_id, "call-1");
    assert_eq!(state.call_type, "create_run");
}
