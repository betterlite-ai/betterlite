#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;

use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_core::api::eval_create_list::config::EvalsConfigLookup;
use betterlite_core::api::eval_lifecycle::EvalLifecycleConfigLookup;
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

// a server that answers one request per connection and then returns the request lines it saw
struct Server {
    base: String,
    worker: JoinHandle<Vec<String>>,
}

impl Server {
    fn start(count: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the test server binds");
        let port = listener
            .local_addr()
            .expect("the test server has an address")
            .port();
        let worker = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for stream in listener.incoming().take(count) {
                let mut stream = stream.expect("the test server accepts");
                let request = read_request(&mut stream);
                let reply = reply_for(&request);
                seen.push(request);
                write_reply(&mut stream, reply);
            }
            seen
        });
        Self {
            base: format!("http://127.0.0.1:{port}"),
            worker,
        }
    }

    fn finish(self) -> Vec<String> {
        self.worker.join().expect("the test server stops")
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().expect("the test server clones"));
    let mut head = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("the test server reads");
        if line == "\r\n" || line.is_empty() {
            break;
        }
        head.push_str(&line);
    }
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .expect("the test server reads the body");
    format!(
        "{head}\r\n{}",
        String::from_utf8(body).expect("the body is JSON")
    )
}

fn reply_for(request: &str) -> &'static str {
    let mut parts = request
        .lines()
        .next()
        .expect("a request has a request line")
        .split(' ');
    let method = parts.next().expect("a request line names the method");
    let target = parts.next().expect("a request line names the path");
    let path = target.split('?').next().expect("a target has a path");
    match (method, path) {
        ("POST", "/v1/evals") => CREATED,
        ("GET", "/v1/evals") => LISTED,
        ("POST", _) if path.ends_with("/cancel") => CANCELLED,
        ("POST", _) => UPDATED,
        ("GET", _) => FETCHED,
        ("DELETE", _) => DELETED,
        (_, _) => panic!("the test server has no reply for {method} {path}"),
    }
}

fn write_reply(stream: &mut TcpStream, body: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .expect("the test server writes");
    stream
        .write_all(body.as_bytes())
        .expect("the test server writes");
    stream.flush().expect("the test server flushes");
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
    let server = Server::start(6);
    let base = server.base.clone();
    Python::attach(|py| {
        let session = NativeEvalSession::new(ENVIRONMENT).expect("the session builds");
        let created = session
            .create_eval(py, &create_call(&base), Vec::new())
            .expect("create answers");
        assert!(created.contains("eval_created"), "{created}");
        let listed = session
            .list_evals(py, &list_call(&base), Vec::new())
            .expect("list answers");
        assert!(listed.contains(r#""object":"list""#), "{listed}");
        let fetched = session
            .get_eval(py, &eval_call(&base), Vec::new())
            .expect("get answers");
        assert!(fetched.contains("eval_fetched"), "{fetched}");
        let updated = session
            .update_eval(py, &update_call(&base), Vec::new())
            .expect("update answers");
        assert!(updated.contains("eval_updated"), "{updated}");
        let deleted = session
            .delete_eval(py, &eval_call(&base), Vec::new())
            .expect("delete answers");
        assert!(deleted.contains(r#""deleted":true"#), "{deleted}");
        let cancelled = session
            .cancel_eval(py, &eval_call(&base), Vec::new())
            .expect("cancel answers");
        assert!(cancelled.contains(r#""status":"cancelled""#), "{cancelled}");
    });
    let seen = server.finish();
    assert!(
        seen[0].starts_with("POST /v1/evals HTTP/1.1"),
        "{}",
        seen[0]
    );
    assert!(
        seen[1].starts_with("GET /v1/evals?limit=5 HTTP/1.1"),
        "{}",
        seen[1]
    );
    assert!(
        seen[2].starts_with("GET /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        seen[2]
    );
    assert!(
        seen[3].starts_with("POST /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        seen[3]
    );
    assert!(
        seen[4].starts_with("DELETE /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        seen[4]
    );
    assert!(
        seen[5].starts_with("POST /v1/evals/eval_1/cancel HTTP/1.1"),
        "{}",
        seen[5]
    );
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
    let server = Server::start(6);
    let base = server.base.clone();
    Python::attach(|py| {
        let session = NativeEvalSession::new(ENVIRONMENT).expect("the session builds");
        let calls = PyList::empty(py);
        let requests = [
            ("acreate_eval", create_call(&base)),
            ("alist_evals", list_call(&base)),
            ("aget_eval", eval_call(&base)),
            ("aupdate_eval", update_call(&base)),
            ("adelete_eval", eval_call(&base)),
            ("acancel_eval", eval_call(&base)),
        ];
        for (method, request) in requests {
            calls
                .append((method, request))
                .expect("the call list grows");
        }
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
        let answers: Vec<String> = answers.extract().expect("the answers are text");
        assert!(answers[0].contains("eval_created"), "{}", answers[0]);
        assert!(answers[1].contains(r#""object":"list""#), "{}", answers[1]);
        assert!(answers[2].contains("eval_fetched"), "{}", answers[2]);
        assert!(answers[3].contains("eval_updated"), "{}", answers[3]);
        assert!(answers[4].contains(r#""deleted":true"#), "{}", answers[4]);
        assert!(
            answers[5].contains(r#""status":"cancelled""#),
            "{}",
            answers[5]
        );
    });
    let seen = server.finish();
    assert!(
        seen[0].starts_with("POST /v1/evals HTTP/1.1"),
        "{}",
        seen[0]
    );
    assert!(
        seen[1].starts_with("GET /v1/evals?limit=5 HTTP/1.1"),
        "{}",
        seen[1]
    );
    assert!(
        seen[2].starts_with("GET /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        seen[2]
    );
    assert!(
        seen[3].starts_with("POST /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        seen[3]
    );
    assert!(
        seen[4].starts_with("DELETE /v1/evals/eval_1 HTTP/1.1"),
        "{}",
        seen[4]
    );
    assert!(
        seen[5].starts_with("POST /v1/evals/eval_1/cancel HTTP/1.1"),
        "{}",
        seen[5]
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
        authorization(&EvalConfigs::new(&both, &environment())).as_deref(),
        Some("Bearer module-key")
    );
    let fallback = ModuleSettings {
        api_key: None,
        openai_key: Some(SecretString::from("openai-key")),
    };
    assert_eq!(
        authorization(&EvalConfigs::new(&fallback, &environment())).as_deref(),
        Some("Bearer openai-key")
    );
    let empty = ModuleSettings {
        api_key: Some(SecretString::from("")),
        openai_key: None,
    };
    assert_eq!(
        authorization(&EvalConfigs::new(&empty, &environment())).as_deref(),
        Some("Bearer env-key")
    );
    let none = ModuleSettings::default();
    let configs = EvalConfigs::new(&none, &BTreeMap::new());
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
    let configs = EvalConfigs::new(&ModuleSettings::default(), &environment());
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
