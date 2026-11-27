#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::agents::collection::AgentFailure;
use betterlite_core::api::agents::item::error::{AgentItemError, ProviderFailure, failure};
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::agents_create_read::logging::{
    AgentLogging, AgentPostCall, AgentPreCall,
};
use betterlite_http::interactions::agents_create_read::{
    AgentRequestError, AgentsApiConfig, CreateAgentRequest, GetAgentRequest, ListAgentsRequest,
    create_sync::create_agent, get_sync::get_agent, list_sync::list_agents,
};
use betterlite_http::interactions::agents_delete_versions::AgentRequest;
use betterlite_http::interactions::agents_delete_versions::synchronous::{
    delete_agent, list_agent_versions,
};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{Logging, Params, RequestHeaders};
use betterlite_http_fixture::error::FixtureError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::GeminiAgentsConfig;
use super::error::GeminiAgentError;
use crate::studio_agents::error::StudioAgentError;

struct Recorder;

impl AgentLogging for Recorder {
    fn pre_call(&mut self, _call: AgentPreCall<'_>) {}

    fn post_call(&mut self, _call: AgentPostCall<'_>) {}
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

fn config(environment_key: Option<&str>) -> GeminiAgentsConfig {
    let environment_key = environment_key.map(secret);
    GeminiAgentsConfig {
        settings: Settings {
            environment_key,
            generic_api_key: None,
            api_base: DEFAULT_API_BASE.to_owned(),
        },
    }
}

fn clients() -> HttpClients {
    HttpClients::new(policy(), proxy_policy())
}

fn policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test/0".to_owned(),
    }
}

fn proxy_policy() -> Arc<ProxyPolicy> {
    let environment = BTreeMap::new();
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
        .expect("the empty proxy environment resolves");
    Arc::new(policy)
}

fn serving(status: u16, body: &str) -> Serving {
    let answer = reply(&format!("{status} fixture"), &[], body);
    serve(answer, Duration::ZERO).expect("the fixture starts")
}

fn params_for(server: &Serving) -> Params {
    let mut params = Params::new();
    let address = server.address;
    let base = format!("http://{address}");
    params.insert("api_base".to_owned(), JsonValue::Text(base));
    params
}

// a custom api_base needs an explicit api_key, so every call that reaches the fixture carries one
fn authenticated(server: &Serving) -> Params {
    let mut params = params_for(server);
    set_text(&mut params, "api_key", "param-key");
    params
}

fn set_text(params: &mut Params, name: &str, value: &str) {
    let text = JsonValue::Text(value.to_owned());
    params.insert(name.to_owned(), text);
}

fn set_number(params: &mut Params, name: &str, value: i64) {
    let number = JsonValue::Number(Number::from(value));
    params.insert(name.to_owned(), number);
}

fn create_request<'call>(name: &'call str, params: &'call Params) -> CreateAgentRequest<'call> {
    CreateAgentRequest {
        name,
        litellm_params: params,
        extra_headers: BTreeMap::new(),
        extra_body: None,
        timeout: None,
        request_timeout: Duration::from_secs(5),
    }
}

fn list_request(params: &Params) -> ListAgentsRequest<'_> {
    ListAgentsRequest {
        litellm_params: params,
        extra_headers: BTreeMap::new(),
        timeout: None,
    }
}

fn parse(text: &str) -> Json {
    serde_json::from_str(text).expect("the text is JSON")
}

fn assert_json(actual: &str, expected: &str) {
    assert_eq!(parse(actual), parse(expected), "{actual}");
}

fn header(head: &str, wanted: &str) -> Option<String> {
    for line in head.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(wanted) {
            return Some(value.trim().to_owned());
        }
    }
    None
}

#[test]
fn create_sends_the_native_body_and_reads_the_agent() {
    let server = serving(200, r#"{"id":"agent-1","base_agent":"waverunner"}"#);
    let mut params = authenticated(&server);
    set_text(&mut params, "base_agent", "waverunner");
    set_text(&mut params, "instructions", "be nice");
    let environment = JsonValue::Object(IndexMap::from([(
        "type".to_owned(),
        JsonValue::Text("remote".to_owned()),
    )]));
    params.insert("base_environment".to_owned(), environment);
    let mut clients = clients();
    let request = create_request("my-agent", &params);
    let agent = create_agent(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect("the create succeeds");
    let sent = server.finish().expect("the fixture saw the request");
    assert_eq!(sent.line, "POST /v1beta/agents HTTP/1.1");
    assert_eq!(
        header(&sent.head, "x-goog-api-key").as_deref(),
        Some("param-key")
    );
    assert_json(
        &sent.body,
        r#"{"name":"my-agent","base_agent":"waverunner","instructions":"be nice","base_environment":{"type":"remote"}}"#,
    );
    assert_eq!(agent.id.as_deref(), Some("agent-1"));
    assert_eq!(agent.name.as_deref(), Some("agent-1"));
    assert_eq!(
        agent.extension["base_agent"],
        Json::Text("waverunner".to_owned())
    );
}

#[test]
fn a_non_json_create_reply_falls_back_to_the_request_name() {
    let server = serving(200, "not json");
    let params = authenticated(&server);
    let mut clients = clients();
    let request = create_request("my-agent", &params);
    let agent = create_agent(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect("the create succeeds");
    server.finish().expect("the fixture saw the request");
    assert_eq!(agent.id.as_deref(), Some("my-agent"));
    assert_eq!(agent.name.as_deref(), Some("my-agent"));
    assert!(agent.extension.is_empty());
}

#[test]
fn list_sends_pagination_and_reads_the_page() {
    let server = serving(
        200,
        r#"{"agents":[{"id":"alpha"}],"nextPageToken":"page-2"}"#,
    );
    let mut params = authenticated(&server);
    set_number(&mut params, "page_size", 25);
    set_text(&mut params, "page_token", "page-1");
    let mut clients = clients();
    let request = list_request(&params);
    let page = list_agents(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect("the list succeeds");
    let sent = server.finish().expect("the fixture saw the request");
    assert_eq!(
        sent.line,
        "GET /v1beta/agents?pageSize=25&pageToken=page-1 HTTP/1.1"
    );
    assert_eq!(page.agents.len(), 1);
    assert_eq!(page.next_page_token.as_deref(), Some("page-2"));
}

#[test]
fn list_omits_empty_pagination() {
    let server = serving(200, r#"{"agents":[]}"#);
    let mut params = authenticated(&server);
    set_number(&mut params, "page_size", 0);
    set_text(&mut params, "page_token", "");
    let mut clients = clients();
    let request = list_request(&params);
    let page = list_agents(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect("the list succeeds");
    let sent = server.finish().expect("the fixture saw the request");
    assert_eq!(sent.line, "GET /v1beta/agents HTTP/1.1");
    assert!(page.agents.is_empty());
    assert_eq!(page.next_page_token, None);
}

#[test]
fn versions_send_pagination_and_read_the_page() {
    let server = serving(
        200,
        r#"{"agentVersions":[{"name":"agents/a/versions/1"}],"nextPageToken":"v-2"}"#,
    );
    let mut params = authenticated(&server);
    set_number(&mut params, "page_size", 3);
    set_text(&mut params, "page_token", "next");
    let mut clients = clients();
    let mut logging = Logging::default();
    let request = AgentRequest {
        name: "a/b",
        litellm_params: &params,
        extra_headers: RequestHeaders::new(),
        timeout: None,
        request_timeout: RequestTimeout {
            seconds: 6000.0,
            explicitly_set: false,
        },
    };
    let page = list_agent_versions(
        &config(Some("env-key")),
        &mut clients,
        &mut logging,
        None,
        &request,
    )
    .expect("the versions list succeeds");
    let sent = server.finish().expect("the fixture saw the request");
    assert_eq!(
        sent.line,
        "GET /v1beta/agents/a/b/versions?pageSize=3&pageToken=next HTTP/1.1"
    );
    assert_eq!(page.agent_versions.len(), 1);
    assert_eq!(page.next_page_token.as_deref(), Some("v-2"));
}

#[test]
fn versions_omit_empty_pagination() {
    let server = serving(200, r#"{"agentVersions":[]}"#);
    let mut params = authenticated(&server);
    set_number(&mut params, "page_size", 0);
    set_text(&mut params, "page_token", "");
    let mut clients = clients();
    let mut logging = Logging::default();
    let request = AgentRequest {
        name: "a/b",
        litellm_params: &params,
        extra_headers: RequestHeaders::new(),
        timeout: None,
        request_timeout: RequestTimeout {
            seconds: 6000.0,
            explicitly_set: false,
        },
    };
    let page = list_agent_versions(
        &config(Some("env-key")),
        &mut clients,
        &mut logging,
        None,
        &request,
    )
    .expect("the versions list succeeds");
    let sent = server.finish().expect("the fixture saw the request");
    assert_eq!(sent.line, "GET /v1beta/agents/a/b/versions HTTP/1.1");
    assert!(page.agent_versions.is_empty());
    assert_eq!(page.next_page_token, None);
}

#[test]
fn get_reads_one_agent() {
    let server = serving(200, r#"{"id":"agent-1","displayName":"helper"}"#);
    let params = authenticated(&server);
    let mut clients = clients();
    let request = GetAgentRequest {
        name: "agent-1",
        litellm_params: &params,
        extra_headers: BTreeMap::new(),
        timeout: None,
    };
    let agent = get_agent(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect("the get succeeds");
    let sent = server.finish().expect("the fixture saw the request");
    assert_eq!(sent.line, "GET /v1beta/agents/agent-1 HTTP/1.1");
    assert_eq!(agent.id.as_deref(), Some("agent-1"));
    assert_eq!(agent.name.as_deref(), Some("agent-1"));
    assert_eq!(
        agent.extension["displayName"],
        Json::Text("helper".to_owned())
    );
}

#[rstest]
#[case::ok(200)]
#[case::no_content(204)]
fn delete_answers_the_name_and_deleted(#[case] status: u16) {
    let server = serving(status, "");
    let params = authenticated(&server);
    let mut clients = clients();
    let mut logging = Logging::default();
    let request = AgentRequest {
        name: "my-agent",
        litellm_params: &params,
        extra_headers: RequestHeaders::new(),
        timeout: None,
        request_timeout: RequestTimeout {
            seconds: 6000.0,
            explicitly_set: false,
        },
    };
    let deleted = delete_agent(
        &config(Some("env-key")),
        &mut clients,
        &mut logging,
        None,
        &request,
    )
    .expect("the delete succeeds");
    let sent = server.finish().expect("the fixture saw the request");
    assert_eq!(sent.line, "DELETE /v1beta/agents/my-agent HTTP/1.1");
    let key = header(&sent.head, "x-goog-api-key");
    assert_eq!(key.as_deref(), Some("param-key"));
    assert_eq!(deleted.name, "my-agent");
    assert!(deleted.deleted);
}

#[test]
fn a_failing_status_becomes_the_gemini_error() {
    let answer = reply("404 fixture", &[("x-goog-request", "r")], "missing agent");
    let server = serve(answer, Duration::ZERO).expect("the fixture starts");
    let params = authenticated(&server);
    let mut clients = clients();
    let request = create_request("my-agent", &params);
    let failure = create_agent(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect_err("a 404 fails");
    server.finish().expect("the fixture saw the request");
    let AgentRequestError::Provider(error) = failure else {
        panic!("a failing status is the provider's error");
    };
    assert_eq!(AgentFailure::status_code(&error), Some(404));
    assert_eq!(AgentFailure::message(&error), "missing agent");
    let GeminiAgentError::Status {
        status,
        message,
        headers,
    } = error
    else {
        panic!("a failing status becomes the Gemini error");
    };
    assert_eq!(status, 404);
    assert_eq!(message, "missing agent");
    assert_eq!(headers.get("x-goog-request").map(String::as_str), Some("r"));
}

#[test]
fn a_get_failure_keeps_the_gemini_studio_error() {
    let answer = reply("404 fixture", &[("x-goog-request", "r")], "gone");
    let server = serve(answer, Duration::ZERO).expect("the fixture starts");
    let params = authenticated(&server);
    let mut clients = clients();
    let request = GetAgentRequest {
        name: "agent-1",
        litellm_params: &params,
        extra_headers: BTreeMap::new(),
        timeout: None,
    };
    let failure = get_agent(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect_err("a 404 fails");
    server.finish().expect("the fixture saw the request");
    let AgentRequestError::Provider(error) = failure else {
        panic!("a failing status is the provider's error");
    };
    let GeminiAgentError::Studio(StudioAgentError::Gemini {
        status,
        body,
        headers,
    }) = &error
    else {
        panic!("a get failure keeps the Gemini body");
    };
    assert_eq!(*status, 404);
    assert_eq!(body.as_str(), "gone");
    let header_value = headers.get("x-goog-request").map(String::as_str);
    assert_eq!(header_value, Some("r"));
    assert_eq!(ProviderFailure::status_code(&error), 404);
    assert_eq!(ProviderFailure::message(&error), "gone");
}

#[test]
fn a_custom_base_without_a_key_is_the_studio_error() {
    let server = serving(200, "{}");
    let params = params_for(&server);
    let mut clients = clients();
    let request = create_request("my-agent", &params);
    let failure = create_agent(&config(None), &mut clients, &mut Recorder, None, &request)
        .expect_err("a custom base needs an explicit key");
    let AgentRequestError::Provider(error) = failure else {
        panic!("the environment is the provider's error");
    };
    assert!(matches!(
        error,
        GeminiAgentError::Studio(StudioAgentError::CustomBaseWithoutKey)
    ));
    assert!(matches!(server.finish(), Err(FixtureError::NoRequest)));
}

#[test]
fn a_wrong_typed_api_key_names_the_field() {
    let server = serving(200, "{}");
    let mut params = authenticated(&server);
    params.insert("api_key".to_owned(), JsonValue::Number(Number::from(5_i64)));
    let mut clients = clients();
    let request = create_request("my-agent", &params);
    let failure = create_agent(&config(None), &mut clients, &mut Recorder, None, &request)
        .expect_err("a number is not a key");
    let AgentRequestError::Provider(error) = failure else {
        panic!("the parameters are the provider's error");
    };
    assert!(matches!(&error, GeminiAgentError::Params { field } if *field == "api_key"));
    assert!(error.to_string().contains("api_key"), "{error}");
    assert!(matches!(server.finish(), Err(FixtureError::NoRequest)));
}

#[test]
fn a_wrong_typed_base_environment_names_the_field() {
    let server = serving(200, "{}");
    let mut params = authenticated(&server);
    let environment = JsonValue::Number(Number::from(1_i64));
    params.insert("base_environment".to_owned(), environment);
    let mut clients = clients();
    let request = create_request("my-agent", &params);
    let failure = create_agent(
        &config(Some("env-key")),
        &mut clients,
        &mut Recorder,
        None,
        &request,
    )
    .expect_err("a number is not a base environment");
    let AgentRequestError::Provider(error) = failure else {
        panic!("the parameters are the provider's error");
    };
    assert!(
        matches!(&error, GeminiAgentError::Params { field } if *field == "base_environment"),
        "{error}"
    );
    assert!(matches!(server.finish(), Err(FixtureError::NoRequest)));
}

#[test]
fn the_environment_key_fills_the_header() {
    let config = config(Some("env-key"));
    let caller = BTreeMap::new();
    let litellm_params = Params::new();
    let headers = AgentsApiConfig::validate_environment(&config, &caller, &litellm_params)
        .expect("the environment key is used");
    let key = headers
        .get("x-goog-api-key")
        .map(SecretString::expose_secret);
    assert_eq!(key, Some("env-key"));
    let content = headers.get("Content-Type").map(SecretString::expose_secret);
    assert_eq!(content, Some("application/json"));
}

#[test]
fn a_missing_key_dispatches_like_a_value_error() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    let error = GeminiAgentError::Studio(StudioAgentError::MissingApiKey);
    let message = error.to_string();
    let raised = failure(
        AgentItemError::Provider(error),
        "agent-1",
        Some("gemini"),
        &redactor,
    );
    let answer = RawException {
        model: "agent-1".to_owned(),
        custom_llm_provider: "gemini".to_owned(),
        error_str: message.clone(),
        status_code: None,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: "ValueError".to_owned(),
    };
    let expected = dispatch(&DispatchInput::Raw(Box::new(answer)), &redactor);
    assert_eq!(raised.class, expected);
    assert_eq!(raised.class.metadata().status_code, 500);
    assert!(matches!(raised.class, ErrorClass::ApiConnection(_)));
}
