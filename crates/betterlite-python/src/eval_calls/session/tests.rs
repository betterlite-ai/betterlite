#![cfg(test)]

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_core::api::eval_create_list::config::EvalsConfigLookup;
use betterlite_core::api::eval_lifecycle::EvalLifecycleConfigLookup;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_models::decisions::Json;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::BaseEvalsAPIConfig;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use secrecy::SecretString;

use super::EvalCore;
use super::NativeEvalSession;
use crate::eval_calls::configs::EvalConfigs;
use crate::eval_calls::convert::ModuleSettings;

const ENVIRONMENT: &str = r#"{"environment":{}}"#;
const CREATED: &str =
    r#"{"id":"eval_created","created_at":1,"data_source_config":{},"testing_criteria":[]}"#;
const FETCHED: &str =
    r#"{"id":"eval_fetched","created_at":1,"data_source_config":{},"testing_criteria":[]}"#;
const UPDATED: &str =
    r#"{"id":"eval_updated","created_at":1,"data_source_config":{},"testing_criteria":[]}"#;
const LISTED: &str = r#"{"object":"list","data":[],"has_more":false}"#;
const DELETED: &str = r#"{"eval_id":"eval_1","deleted":true}"#;
const CANCELLED: &str = r#"{"id":"eval_1","status":"cancelled"}"#;

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
        r#"{{"data_source_config":{{"type":"custom","item_schema":{{}}}},"testing_criteria":[],"api_base":"{base}","api_key":"sk-test"}}"#
    )
}

fn eval_call(base: &str) -> String {
    call(base, r#""eval_id":"eval_1","#)
}

fn update_call(base: &str) -> String {
    call(base, r#""eval_id":"eval_1","name":"probe","#)
}

fn list_call(base: &str) -> String {
    call(base, r#""limit":5,"#)
}

fn message(py: Python<'_>, error: PyErr) -> String {
    error.value(py).to_string()
}

#[test]
fn register_adds_the_eval_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").expect("the probe module exists");
        super::super::register(&module).expect("register succeeds");
        assert!(
            module
                .hasattr("NativeEvalSession")
                .expect("the module reads")
        );
        assert!(module.hasattr("NativeEvalError").expect("the module reads"));
    });
}

#[test]
fn session_bad_settings_text_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let error = NativeEvalSession::new("not json")
            .err()
            .expect("settings must parse");
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn session_bad_json_is_a_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEvalSession::new(ENVIRONMENT).expect("the session builds");
        let error = session
            .create_eval(py, "not json", Vec::new())
            .err()
            .expect("a bad payload must fail");
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn session_sync_calls_reach_their_own_operation() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEvalSession::new(ENVIRONMENT).expect("the session builds");
        let created = serve_answer(CREATED);
        let created_answer = session
            .create_eval(py, &create_call(&base(&created)), Vec::new())
            .expect("create answers");
        assert!(created_answer.contains("eval_created"), "{created_answer}");
        let created_request = created.finish().expect("the fixture answers once");
        assert!(
            created_request.line.starts_with("POST /v1/evals HTTP/1.1"),
            "{}",
            created_request.line
        );
        let listed = serve_answer(LISTED);
        let listed_answer = session
            .list_evals(py, &list_call(&base(&listed)), Vec::new())
            .expect("list answers");
        assert!(
            listed_answer.contains(r#""object":"list""#),
            "{listed_answer}"
        );
        let listed_request = listed.finish().expect("the fixture answers once");
        assert!(
            listed_request
                .line
                .starts_with("GET /v1/evals?limit=5 HTTP/1.1"),
            "{}",
            listed_request.line
        );
        let fetched = serve_answer(FETCHED);
        let fetched_answer = session
            .get_eval(py, &eval_call(&base(&fetched)), Vec::new())
            .expect("get answers");
        assert!(fetched_answer.contains("eval_fetched"), "{fetched_answer}");
        let fetched_request = fetched.finish().expect("the fixture answers once");
        assert!(
            fetched_request
                .line
                .starts_with("GET /v1/evals/eval_1 HTTP/1.1"),
            "{}",
            fetched_request.line
        );
        let updated = serve_answer(UPDATED);
        let updated_answer = session
            .update_eval(py, &update_call(&base(&updated)), Vec::new())
            .expect("update answers");
        assert!(updated_answer.contains("eval_updated"), "{updated_answer}");
        let updated_request = updated.finish().expect("the fixture answers once");
        assert!(
            updated_request
                .line
                .starts_with("POST /v1/evals/eval_1 HTTP/1.1"),
            "{}",
            updated_request.line
        );
        let deleted = serve_answer(DELETED);
        let deleted_answer = session
            .delete_eval(py, &eval_call(&base(&deleted)), Vec::new())
            .expect("delete answers");
        assert!(
            deleted_answer.contains(r#""deleted":true"#),
            "{deleted_answer}"
        );
        let deleted_request = deleted.finish().expect("the fixture answers once");
        assert!(
            deleted_request
                .line
                .starts_with("DELETE /v1/evals/eval_1 HTTP/1.1"),
            "{}",
            deleted_request.line
        );
        let cancelled = serve_answer(CANCELLED);
        let cancelled_answer = session
            .cancel_eval(py, &eval_call(&base(&cancelled)), Vec::new())
            .expect("cancel answers");
        assert!(
            cancelled_answer.contains(r#""status":"cancelled""#),
            "{cancelled_answer}"
        );
        let cancelled_request = cancelled.finish().expect("the fixture answers once");
        assert!(
            cancelled_request
                .line
                .starts_with("POST /v1/evals/eval_1/cancel HTTP/1.1"),
            "{}",
            cancelled_request.line
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

#[test]
fn session_async_calls_reach_their_own_operation() {
    Python::initialize();
    let created = serve_answer(CREATED);
    let listed = serve_answer(LISTED);
    let fetched = serve_answer(FETCHED);
    let updated = serve_answer(UPDATED);
    let deleted = serve_answer(DELETED);
    let cancelled = serve_answer(CANCELLED);
    Python::attach(|py| {
        let session = NativeEvalSession::new(ENVIRONMENT).expect("the session builds");
        let calls = PyList::empty(py);
        calls
            .append(("acreate_eval", create_call(&base(&created))))
            .expect("the call list grows");
        calls
            .append(("alist_evals", list_call(&base(&listed))))
            .expect("the call list grows");
        calls
            .append(("aget_eval", eval_call(&base(&fetched))))
            .expect("the call list grows");
        calls
            .append(("aupdate_eval", update_call(&base(&updated))))
            .expect("the call list grows");
        calls
            .append(("adelete_eval", eval_call(&base(&deleted))))
            .expect("the call list grows");
        calls
            .append(("acancel_eval", eval_call(&base(&cancelled))))
            .expect("the call list grows");
        let globals = PyDict::new(py);
        globals
            .set_item("session", session)
            .expect("the globals take the session");
        globals
            .set_item("calls", calls)
            .expect("the globals take the calls");
        py.run(AWAIT_CALLS, Some(&globals), None)
            .expect("the awaits finish");
        let answers = globals
            .get_item("answers")
            .expect("the globals hold the answers")
            .expect("the answers were set");
        let texts: Vec<String> = answers.extract().expect("the answers are text");
        assert!(texts[0].contains("eval_created"), "{}", texts[0]);
        assert!(texts[1].contains(r#""object":"list""#), "{}", texts[1]);
        assert!(texts[2].contains("eval_fetched"), "{}", texts[2]);
        assert!(texts[3].contains("eval_updated"), "{}", texts[3]);
        assert!(texts[4].contains(r#""deleted":true"#), "{}", texts[4]);
        assert!(texts[5].contains(r#""status":"cancelled""#), "{}", texts[5]);
    });
    let created_request = created.finish().expect("the fixture answers once");
    assert!(
        created_request.line.starts_with("POST /v1/evals HTTP/1.1"),
        "{}",
        created_request.line
    );
    let listed_request = listed.finish().expect("the fixture answers once");
    assert!(
        listed_request
            .line
            .starts_with("GET /v1/evals?limit=5 HTTP/1.1"),
        "{}",
        listed_request.line
    );
    let fetched_request = fetched.finish().expect("the fixture answers once");
    assert!(
        fetched_request
            .line
            .starts_with("GET /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        fetched_request.line
    );
    let updated_request = updated.finish().expect("the fixture answers once");
    assert!(
        updated_request
            .line
            .starts_with("POST /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        updated_request.line
    );
    let deleted_request = deleted.finish().expect("the fixture answers once");
    assert!(
        deleted_request
            .line
            .starts_with("DELETE /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        deleted_request.line
    );
    let cancelled_request = cancelled.finish().expect("the fixture answers once");
    assert!(
        cancelled_request
            .line
            .starts_with("POST /v1/evals/eval_1/cancel HTTP/1.1"),
        "{}",
        cancelled_request.line
    );
}

#[test]
fn session_a_wrong_typed_field_is_a_bad_request() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeEvalSession::new(ENVIRONMENT).expect("the session builds");
        let error = session
            .list_evals(py, r#"{"limit": "many"}"#, Vec::new())
            .err()
            .expect("a wrong-typed limit must fail");
        let text = message(py, error);
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
        let session = NativeEvalSession::new(ENVIRONMENT).expect("the session builds");
        let error = session
            .get_eval(py, &eval_call(&base), Vec::new())
            .err()
            .expect("a refused connection must fail");
        let text = message(py, error);
        assert!(text.contains(r#""type":"InternalServerError""#), "{text}");
        assert!(text.contains(r#""status_code":500"#), "{text}");
        assert!(text.contains(r#""llm_provider":"openai""#), "{text}");
    });
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([("OPENAI_API_KEY".to_owned(), "env-key".to_owned())])
}

fn authorization(configs: &EvalConfigs) -> Option<String> {
    let lookup: &dyn EvalsConfigLookup = configs;
    let config = lookup
        .evals_config(EvalsConfigTag::OpenAI)
        .expect("openai has a config");
    let headers = config
        .validate_environment(Headers::new(), None)
        .expect("a key resolves");
    headers.get("Authorization").cloned()
}

#[test]
fn configs_read_the_module_key_before_the_openai_key_and_the_environment() {
    let both = ModuleSettings {
        api_key: Some(SecretString::from("module-key")),
        openai_key: Some(SecretString::from("openai-key")),
    };
    assert_eq!(
        authorization(&EvalConfigs::new(both, &environment())).as_deref(),
        Some("Bearer module-key")
    );
    let fallback = ModuleSettings {
        api_key: None,
        openai_key: Some(SecretString::from("openai-key")),
    };
    assert_eq!(
        authorization(&EvalConfigs::new(fallback, &environment())).as_deref(),
        Some("Bearer openai-key")
    );
    let empty = ModuleSettings {
        api_key: Some(SecretString::from("")),
        openai_key: None,
    };
    assert_eq!(
        authorization(&EvalConfigs::new(empty, &environment())).as_deref(),
        Some("Bearer env-key")
    );
    let none = ModuleSettings::default();
    let configs = EvalConfigs::new(none, &BTreeMap::new());
    let lookup: &dyn EvalsConfigLookup = &configs;
    let config = lookup
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
    let state = super::logging::call_logging(&core, "create_eval", &kwargs)
        .expect("the logging state builds");
    assert_eq!(state.litellm_call_id, "call-1");
    assert_eq!(state.call_type, "create_eval");
}
