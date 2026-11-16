#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::endpoint::{AgentEndpoint, endpoint_url};
use super::error::{AgentRequestError, check_response};
use super::params::SUPPORTED_OPENAI_PARAMS;
use super::prepare::{Headers, PreparedAgentCall, prepare, validate_environment};
use super::request::{
    AgentCallRequest, DEFAULT_API_VERSION, agent_id, agent_id_from_model, api_version,
    complete_url, provider_info, transform_request,
};
use super::stream::{HAS_CUSTOM_STREAM_WRAPPER, SHOULD_FAKE_STREAM, SUPPORTS_STREAM_PARAM_IN_BODY};
use crate::credentials::{AZURE_AI_API_BASE_VAR, AZURE_AI_API_KEY_VAR};

const SENTINEL: &str = "sentinel-fixture-credential";
const API_BASE_REQUIRED: &str = "api_base is required for Azure AI Agents. Set it via AZURE_AI_API_BASE env var or api_base parameter.";

static EMPTY_HEADERS: Headers = BTreeMap::new();

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no network in tests".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn serialized<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn strings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn header_map(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

fn shown(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

fn request<'data>(
    model: &'data str,
    optional_params: &'data Object,
    messages: &'data [Object],
    request_headers: &'data Headers,
) -> AgentCallRequest<'data> {
    AgentCallRequest {
        model,
        messages,
        optional_params,
        headers: request_headers,
        api_key: None,
        api_base: None,
    }
}

fn prepared(
    model: &str,
    optional_params: &Object,
    key: Option<&SecretString>,
) -> Result<PreparedAgentCall, AgentRequestError> {
    let mut call = request(model, optional_params, &[], &EMPTY_HEADERS);
    call.api_base = Some("https://base.example");
    call.api_key = key;
    prepare(&call)
}

fn agent_models_from_model() {
    let empty = Object::new();
    assert_eq!(
        agent_id("azure_ai/agents/agent-one", &empty).unwrap(),
        "agent-one"
    );
    assert_eq!(
        agent_id("agents/first/second", &empty).unwrap(),
        "first/second"
    );
    assert_eq!(agent_id("plain-model", &empty).unwrap(), "plain-model");
    assert_eq!(
        agent_id("azure_ai/Agents/Upper", &empty).unwrap(),
        "azure_ai/Agents/Upper"
    );
    assert_eq!(agent_id_from_model("azure_ai/agents/a/b"), "a/b");
}

fn agent_id_field_precedence() {
    let chosen = object(r#"{"agent_id":"chosen","assistant_id":"other"}"#);
    assert_eq!(
        agent_id("azure_ai/agents/ghost", &chosen).unwrap(),
        "chosen"
    );
    let blank = object(r#"{"agent_id":"","assistant_id":"assistant-one"}"#);
    assert_eq!(
        agent_id("azure_ai/agents/ghost", &blank).unwrap(),
        "assistant-one"
    );
    let zero = object(r#"{"agent_id":0,"assistant_id":"from-assistant"}"#);
    assert_eq!(
        agent_id("azure_ai/agents/ghost", &zero).unwrap(),
        "from-assistant"
    );
    assert!(matches!(
        agent_id("model", &object(r#"{"agent_id":5}"#)),
        Err(AgentRequestError::FieldType { field: "agent_id" })
    ));
}

fn agent_params_passthrough() {
    // the agent route advertises only stream; the request keeps optional_params as given (the oracle case proves it)
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["stream"]);
}

fn agent_model_id() {
    agent_models_from_model();
    agent_id_field_precedence();
    agent_params_passthrough();
}

fn assistant_id_override() {
    let empty = Object::new();
    let assistant = object(r#"{"assistant_id":"assistant-id"}"#);
    assert_eq!(
        agent_id("azure_ai/agents/suffix", &assistant).unwrap(),
        "assistant-id"
    );
    assert_eq!(agent_id("no-slash", &assistant).unwrap(), "assistant-id");
    let both = object(r#"{"agent_id":"agent-id","assistant_id":"assistant-id"}"#);
    assert_eq!(
        agent_id("azure_ai/agents/suffix", &both).unwrap(),
        "agent-id"
    );
    let nulls = object(r#"{"agent_id":null,"assistant_id":null}"#);
    assert_eq!(
        agent_id("azure_ai/agents/suffix", &nulls).unwrap(),
        "suffix"
    );
    let falsy = object(r#"{"agent_id":false,"assistant_id":"assistant-id"}"#);
    assert_eq!(
        agent_id("azure_ai/agents/suffix", &falsy).unwrap(),
        "assistant-id"
    );
    let empty_list = object(r#"{"agent_id":[],"assistant_id":"assistant-id"}"#);
    assert_eq!(
        agent_id("azure_ai/agents/suffix", &empty_list).unwrap(),
        "assistant-id"
    );
    assert_eq!(
        agent_id("azure_ai/agents/suffix", &empty).unwrap(),
        "suffix"
    );
    assert!(matches!(
        agent_id("model", &object(r#"{"assistant_id":["a"]}"#)),
        Err(AgentRequestError::FieldType {
            field: "assistant_id"
        })
    ));
}

fn version_defaults() {
    assert_eq!(DEFAULT_API_VERSION, "2025-05-01");
    assert_eq!(
        api_version(&Object::new()),
        Json::Text(DEFAULT_API_VERSION.to_owned())
    );
    assert_eq!(api_version(&object(r#"{"api_version":null}"#)), Json::Null);
    assert_eq!(
        api_version(&object(r#"{"api_version":""}"#)),
        Json::Text(String::new())
    );
    assert_eq!(
        api_version(&object(r#"{"api_version":"2025-05-15-preview"}"#)),
        Json::Text("2025-05-15-preview".to_owned())
    );
}

fn provider_lookup() {
    let lookup = secrets();
    let explicit_key = SecretString::from("explicit-key");
    let both = strings(&[
        (AZURE_AI_API_BASE_VAR, "https://env.example"),
        (AZURE_AI_API_KEY_VAR, "env-key"),
    ]);
    let explicit = provider_info(
        Some("https://explicit.example"),
        Some(&explicit_key),
        &lookup,
        &both,
    )
    .unwrap();
    assert_eq!(
        explicit.api_base.as_deref(),
        Some("https://explicit.example")
    );
    assert_eq!(shown(explicit.api_key.as_ref()), Some("explicit-key"));
    let fallback = provider_info(None, None, &lookup, &both).unwrap();
    assert_eq!(fallback.api_base.as_deref(), Some("https://env.example"));
    assert_eq!(shown(fallback.api_key.as_ref()), Some("env-key"));
    let empty_key = SecretString::from("");
    let empty_explicit = provider_info(Some(""), Some(&empty_key), &lookup, &both).unwrap();
    assert_eq!(
        empty_explicit.api_base.as_deref(),
        Some("https://env.example")
    );
    assert_eq!(shown(empty_explicit.api_key.as_ref()), Some("env-key"));
    let boolean = strings(&[(AZURE_AI_API_BASE_VAR, "TRUE")]);
    let from_boolean = provider_info(None, None, &lookup, &boolean).unwrap();
    assert_eq!(from_boolean.api_base, None);
    assert_eq!(shown(from_boolean.api_key.as_ref()), None);
    let empty_value = strings(&[(AZURE_AI_API_KEY_VAR, "")]);
    let from_empty = provider_info(None, None, &lookup, &empty_value).unwrap();
    assert_eq!(from_empty.api_base, None);
    assert_eq!(shown(from_empty.api_key.as_ref()), Some(""));
}

fn url_trimming() {
    assert_eq!(
        complete_url(Some("https://base.example/")).unwrap(),
        "https://base.example"
    );
    assert_eq!(
        complete_url(Some("https://base.example///")).unwrap(),
        "https://base.example"
    );
    assert_eq!(complete_url(Some("")).unwrap(), "");
}

fn default_api_version() {
    version_defaults();
    provider_lookup();
    url_trimming();
}

fn thread_payloads() {
    let empty: Headers = BTreeMap::new();
    let base =
        transform_request(&request("azure_ai/agents/a", &Object::new(), &[], &empty)).unwrap();
    assert_eq!(base.thread_id, None);
    assert_eq!(base.instructions, None);
    assert_eq!(
        serialized(&base),
        json(r#"{"agent_id":"a","messages":[],"api_version":"2025-05-01"}"#)
    );
    let thread = object(r#"{"thread_id":"thread-1"}"#);
    let continued = transform_request(&request("azure_ai/agents/a", &thread, &[], &empty)).unwrap();
    assert_eq!(continued.thread_id, Some(Json::Text("thread-1".to_owned())));
    assert_eq!(
        serialized(&continued),
        json(r#"{"agent_id":"a","messages":[],"api_version":"2025-05-01","thread_id":"thread-1"}"#)
    );
    let null_thread = object(r#"{"thread_id":null}"#);
    let preserved =
        transform_request(&request("azure_ai/agents/a", &null_thread, &[], &empty)).unwrap();
    assert_eq!(preserved.thread_id, Some(Json::Null));
    assert_eq!(
        serialized(&preserved),
        json(r#"{"agent_id":"a","messages":[],"api_version":"2025-05-01","thread_id":null}"#)
    );
    let instructions = object(r#"{"instructions":"be brief"}"#);
    let with_instructions =
        transform_request(&request("azure_ai/agents/a", &instructions, &[], &empty)).unwrap();
    assert_eq!(
        with_instructions.instructions,
        Some(Json::Text("be brief".to_owned()))
    );
    assert_eq!(
        serialized(&with_instructions),
        json(
            r#"{"agent_id":"a","messages":[],"api_version":"2025-05-01","instructions":"be brief"}"#
        )
    );
}

fn thread_preparation() {
    let thread = object(r#"{"thread_id":"thread-1"}"#);
    let continued = prepared("azure_ai/agents/a", &thread, None).unwrap();
    assert_eq!(continued.thread_id.as_deref(), Some("thread-1"));
    let null_thread = object(r#"{"thread_id":null}"#);
    let fresh = prepared("azure_ai/agents/a", &null_thread, None).unwrap();
    assert_eq!(fresh.thread_id, None);
    let empty_thread = object(r#"{"thread_id":""}"#);
    let blank = prepared("azure_ai/agents/a", &empty_thread, None).unwrap();
    assert_eq!(blank.thread_id.as_deref(), Some(""));
}

fn existing_thread() {
    thread_payloads();
    thread_preparation();
}

fn message_content_shapes() {
    let messages = [
        object(r#"{"role":"user","content":"hello"}"#),
        object(r#"{"content":"no role"}"#),
        object(r#"{"role":"assistant"}"#),
        object(r#"{"role":"user","content":null}"#),
        object(r#"{"role":"user","content":5}"#),
        object(r#"{"role":"user","content":true}"#),
        object(r#"{"role":"user","content":1.5}"#),
        object(r#"{"role":"user","content":""}"#),
        object(r#"{"role":null,"content":"x"}"#),
        object(
            r#"{"role":"user","content":[{"type":"text","text":"one"},{"type":"text","text":"two"}]}"#,
        ),
        object(
            r#"{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://image.example"}}]}"#,
        ),
        object(r#"{"role":"user","content":[{"type":"text","text":""},{"type":"text"}]}"#),
        object(r#"{"role":"user","content":[]}"#),
    ];
    let empty: Headers = BTreeMap::new();
    let payload = transform_request(&request(
        "azure_ai/agents/a",
        &Object::new(),
        &messages,
        &empty,
    ))
    .unwrap();
    assert_eq!(
        serialized(&payload.messages),
        json(
            r#"[{"role":"user","content":"hello"},{"role":"user","content":"no role"},{"role":"assistant","content":""},{"role":"user","content":"None"},{"role":"user","content":"5"},{"role":"user","content":"True"},{"role":"user","content":"1.5"},{"role":"user","content":""},{"role":null,"content":"x"},{"role":"user","content":"onetwo"},{"role":"user","content":""},{"role":"user","content":""},{"role":"user","content":""}]"#,
        )
    );
}

fn message_search_results() {
    let empty: Headers = BTreeMap::new();
    let mut sourced = object(r#"{"role":"user","content":[{"type":"text","text":"base"}]}"#);
    sourced.insert(
        String::from("search_results"),
        json(r#"[{"source":"src","title":"title"}]"#),
    );
    let converted = transform_request(&request(
        "azure_ai/agents/a",
        &Object::new(),
        &[sourced],
        &empty,
    ))
    .unwrap();
    assert_eq!(converted.messages[0].content, "basesrctitle");
}

fn message_content_conversion() {
    message_content_shapes();
    message_search_results();
}

fn thread_urls() {
    let base = "https://base.example";
    assert_eq!(
        endpoint_url(AgentEndpoint::Threads, base, DEFAULT_API_VERSION).unwrap(),
        "https://base.example/threads?api-version=2025-05-01"
    );
    let spaced = AgentEndpoint::Messages {
        thread_id: "thread 1/2",
    };
    assert_eq!(
        endpoint_url(spaced, base, DEFAULT_API_VERSION).unwrap(),
        "https://base.example/threads/thread%201%2F2/messages?api-version=2025-05-01"
    );
    let unreserved = AgentEndpoint::Runs {
        thread_id: "a.b-c_d~e",
    };
    assert_eq!(
        endpoint_url(unreserved, base, "").unwrap(),
        "https://base.example/threads/a.b-c_d~e/runs?api-version="
    );
    assert_eq!(
        endpoint_url(AgentEndpoint::ThreadAndRun, base, DEFAULT_API_VERSION).unwrap(),
        "https://base.example/threads/runs?api-version=2025-05-01"
    );
    let trailing = AgentEndpoint::Messages { thread_id: "t" };
    assert_eq!(
        endpoint_url(trailing, "https://base.example/", DEFAULT_API_VERSION).unwrap(),
        "https://base.example//threads/t/messages?api-version=2025-05-01"
    );
}

fn run_urls() {
    let base = "https://base.example";
    let status = AgentEndpoint::RunStatus {
        thread_id: "thread-1",
        run_id: "run/1",
    };
    assert_eq!(
        endpoint_url(status, base, "2025-05-15-preview").unwrap(),
        "https://base.example/threads/thread-1/runs/run%2F1?api-version=2025-05-15-preview"
    );
    let missing = endpoint_url(AgentEndpoint::Messages { thread_id: "" }, base, "").unwrap_err();
    assert_eq!(missing.to_string(), "thread_id is required");
    let dot = endpoint_url(AgentEndpoint::Runs { thread_id: "." }, base, "").unwrap_err();
    assert_eq!(dot.to_string(), "thread_id cannot be a dot path segment");
    let dot_run = endpoint_url(
        AgentEndpoint::RunStatus {
            thread_id: "thread-1",
            run_id: "..",
        },
        base,
        "",
    )
    .unwrap_err();
    assert_eq!(dot_run.to_string(), "run_id cannot be a dot path segment");
}

fn thread_and_run_paths() {
    thread_urls();
    run_urls();
}

fn missing_api_base() {
    let missing = complete_url(None).unwrap_err();
    assert_eq!(missing.to_string(), API_BASE_REQUIRED);
    assert_eq!(missing.status(), None);
    let no_params = Object::new();
    let mut call = request("azure_ai/agents/a", &no_params, &[], &EMPTY_HEADERS);
    assert!(matches!(
        prepare(&call),
        Err(AgentRequestError::ApiBaseRequired)
    ));
    call.api_base = Some("https://base.example///");
    assert_eq!(prepare(&call).unwrap().api_base, "https://base.example");
}

fn validation_headers() {
    let key = SecretString::from("secret");
    let validated = validate_environment(&header_map(&[]), Some(&key));
    assert_eq!(
        shown(validated.get("Content-Type")),
        Some("application/json")
    );
    assert_eq!(shown(validated.get("Authorization")), Some("Bearer secret"));
    let existing = header_map(&[
        ("Authorization", "Bearer old"),
        ("Content-Type", "text/plain"),
    ]);
    let preserved = validate_environment(&existing, None);
    assert_eq!(
        shown(preserved.get("Content-Type")),
        Some("application/json")
    );
    assert_eq!(shown(preserved.get("Authorization")), Some("Bearer old"));
    assert_eq!(shown(existing.get("Content-Type")), Some("text/plain"));
    let empty_key = SecretString::from("");
    let empty = validate_environment(&existing, Some(&empty_key));
    assert_eq!(shown(empty.get("Authorization")), Some("Bearer old"));
    let replaced = validate_environment(&existing, Some(&key));
    assert_eq!(shown(replaced.get("Authorization")), Some("Bearer secret"));
    let lower = validate_environment(&header_map(&[("content-type", "text/plain")]), None);
    assert_eq!(shown(lower.get("content-type")), Some("text/plain"));
    assert_eq!(shown(lower.get("Content-Type")), Some("application/json"));
}

fn preparation_identity() {
    let key = SecretString::from("secret");
    let params = object(r#"{"api_version":"2025-05-15-preview","thread_id":"thread-1"}"#);
    let call = prepared("azure_ai/agents/a", &params, Some(&key)).unwrap();
    assert_eq!(call.api_base, "https://base.example");
    assert_eq!(call.api_version, "2025-05-15-preview");
    assert_eq!(call.agent_id, "a");
    assert_eq!(call.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(
        shown(call.headers.get("Authorization")),
        Some("Bearer secret")
    );
    assert_eq!(
        shown(call.headers.get("Content-Type")),
        Some("application/json")
    );
}

fn preparation_version_rendering() {
    let null_version = prepared(
        "azure_ai/agents/a",
        &object(r#"{"api_version":null}"#),
        None,
    );
    assert_eq!(null_version.unwrap().api_version, "None");
    let empty_version = prepared("azure_ai/agents/a", &object(r#"{"api_version":""}"#), None);
    assert_eq!(empty_version.unwrap().api_version, "");
    let number_version = prepared(
        "azure_ai/agents/a",
        &object(r#"{"api_version":2025}"#),
        None,
    );
    assert_eq!(number_version.unwrap().api_version, "2025");
}

fn preparation_type_errors() {
    assert!(matches!(
        prepared("azure_ai/agents/a", &object(r#"{"api_version":[]}"#), None),
        Err(AgentRequestError::FieldType {
            field: "api_version"
        })
    ));
    assert!(matches!(
        prepared("azure_ai/agents/a", &object(r#"{"thread_id":5}"#), None),
        Err(AgentRequestError::FieldType { field: "thread_id" })
    ));
}

fn preparation_fields() {
    preparation_identity();
    preparation_version_rendering();
    preparation_type_errors();
}

fn response_checks() {
    check_response(200, "ok", &[200], "create-thread").unwrap();
    check_response(202, "accepted", &[200, 202], "add-message").unwrap();
    let failed = check_response(404, "not-found", &[200], "create-thread").unwrap_err();
    assert_eq!(failed.to_string(), "create-thread: not-found");
    assert_eq!(failed.status(), Some(404));
    let unexpected = check_response(400, "", &[], "poll-run").unwrap_err();
    assert_eq!(unexpected.to_string(), "poll-run: ");
    let conflict = check_response(409, "conflict", &[200, 202], "poll-run").unwrap_err();
    assert_eq!(conflict.status(), Some(409));
}

fn error_class_and_status() {
    // get_error_class drops the headers it is handed, so only the status and the message survive
    let error = AgentRequestError::Upstream {
        status: 401,
        message: String::from("unauthorized"),
    };
    assert_eq!(error.status(), Some(401));
    assert_eq!(error.to_string(), "unauthorized");
    let failure = prepare(&request(
        "azure_ai/agents/a",
        &Object::new(),
        &[],
        &EMPTY_HEADERS,
    ))
    .unwrap_err();
    assert_eq!(failure.status(), None);
    assert_eq!(failure.to_string(), API_BASE_REQUIRED);
}

fn agent_auth_and_response_checks() {
    validation_headers();
    preparation_fields();
    response_checks();
    error_class_and_status();
}

fn agent_stream_flags() {
    let flags = [
        SHOULD_FAKE_STREAM,
        HAS_CUSTOM_STREAM_WRAPPER,
        SUPPORTS_STREAM_PARAM_IN_BODY,
    ];
    assert_eq!(flags, [true, false, false]);
}

#[rstest]
#[case::agent_model_id(agent_model_id)]
#[case::assistant_id_override(assistant_id_override)]
#[case::default_api_version(default_api_version)]
#[case::existing_thread(existing_thread)]
#[case::message_content_conversion(message_content_conversion)]
#[case::thread_and_run_paths(thread_and_run_paths)]
#[case::missing_api_base(missing_api_base)]
#[case::agent_auth_and_response_checks(agent_auth_and_response_checks)]
#[case::agent_stream_flags(agent_stream_flags)]
fn agent_requests_cases(#[case] check: fn()) {
    check();
}

fn prepared_identity_isolation() {
    let empty: Headers = BTreeMap::new();
    let no_params = Object::new();
    let first_key = SecretString::from("first-key");
    let second_key = SecretString::from("second-key");
    let mut first_call = request("azure_ai/agents/first", &no_params, &[], &empty);
    first_call.api_base = Some("https://first.example");
    first_call.api_key = Some(&first_key);
    let mut second_call = request("azure_ai/agents/second", &no_params, &[], &empty);
    second_call.api_base = Some("https://second.example");
    second_call.api_key = Some(&second_key);
    let first = prepare(&first_call).unwrap();
    let second = prepare(&second_call).unwrap();
    assert_eq!(first.agent_id, "first");
    assert_eq!(second.agent_id, "second");
    assert_eq!(first.api_base, "https://first.example");
    assert_eq!(second.api_base, "https://second.example");
    assert_eq!(
        shown(first.headers.get("Authorization")),
        Some("Bearer first-key")
    );
    assert_eq!(
        shown(second.headers.get("Authorization")),
        Some("Bearer second-key")
    );
    assert!(empty.is_empty());
}

fn environment_identity_isolation() {
    let lookup = secrets();
    let first_key = SecretString::from("first-key");
    let env_first = strings(&[
        (AZURE_AI_API_BASE_VAR, "https://env-first.example"),
        (AZURE_AI_API_KEY_VAR, "env-first-key"),
    ]);
    let env_second = strings(&[
        (AZURE_AI_API_BASE_VAR, "https://env-second.example"),
        (AZURE_AI_API_KEY_VAR, "env-second-key"),
    ]);
    let from_first = provider_info(None, None, &lookup, &env_first).unwrap();
    let from_second = provider_info(None, None, &lookup, &env_second).unwrap();
    assert_eq!(
        from_first.api_base.as_deref(),
        Some("https://env-first.example")
    );
    assert_eq!(
        from_second.api_base.as_deref(),
        Some("https://env-second.example")
    );
    assert_eq!(shown(from_first.api_key.as_ref()), Some("env-first-key"));
    assert_eq!(shown(from_second.api_key.as_ref()), Some("env-second-key"));
    let explicit_key = provider_info(
        Some("https://explicit.example"),
        Some(&first_key),
        &lookup,
        &env_second,
    )
    .unwrap();
    assert_eq!(
        explicit_key.api_base.as_deref(),
        Some("https://explicit.example")
    );
    assert_eq!(shown(explicit_key.api_key.as_ref()), Some("first-key"));
}

#[test]
fn foundry_agents_credential_identity_isolation() {
    prepared_identity_isolation();
    environment_identity_isolation();
}

#[test]
fn foundry_agents_secret_safe_failure() {
    let key = SecretString::from(SENTINEL);
    let params = object(r#"{"api_version":"2025-05-15-preview"}"#);
    let call = prepared("azure_ai/agents/a", &params, Some(&key)).unwrap();
    assert_eq!(
        shown(call.headers.get("Authorization")),
        Some("Bearer sentinel-fixture-credential")
    );
    assert!(!format!("{call:?}").contains(SENTINEL));
    let info = provider_info(
        Some("https://base.example"),
        Some(&key),
        &secrets(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(shown(info.api_key.as_ref()), Some(SENTINEL));
    assert!(!format!("{info:?}").contains(SENTINEL));
    let passed = header_map(&[
        ("Authorization", "Bearer sentinel-fixture-credential"),
        ("X-Trace", "trace-1"),
    ]);
    let mut incoming = request("azure_ai/agents/a", &params, &[], &passed);
    incoming.api_base = Some("https://base.example");
    assert!(!format!("{incoming:?}").contains(SENTINEL));
    let prepared_from_incoming = prepare(&incoming).unwrap();
    assert_eq!(
        shown(prepared_from_incoming.headers.get("Authorization")),
        Some("Bearer sentinel-fixture-credential")
    );
    assert!(!format!("{prepared_from_incoming:?}").contains(SENTINEL));
    let mut incomplete = request("azure_ai/agents/a", &params, &[], &EMPTY_HEADERS);
    incomplete.api_key = Some(&key);
    let failure = prepare(&incomplete).unwrap_err();
    assert!(!format!("{failure:?}").contains(SENTINEL));
    assert!(!format!("{failure}").contains(SENTINEL));
}
