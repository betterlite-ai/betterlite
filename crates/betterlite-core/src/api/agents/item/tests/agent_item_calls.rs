#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind, Read as _, Result as IoResult, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::agents_create_read::AgentsApiConfig;
use betterlite_http::interactions::agents_delete_versions::{
    AgentRequest, AgentsConfig, EnvironmentRequest, NamedAgentRequest,
};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{
    ErrorClasses, Logging, Params, PreparedRequest, RequestHeaders, ResponseHeaders,
    UpstreamResponse,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::error::ProviderFailure;
use super::{
    AgentItemDeps, AgentItemRequest, adelete, aget, alist_versions, delete, get, list_versions,
};
use crate::agents::json::AgentJson;
use crate::agents::provider::{
    AgentCreateResponse, AgentDeleteResult, AgentVersionsResponse, Deleted,
};

const TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(10);
const GET_BODY: &str = r#"{"id": "agent-1"}"#;

// one request the fixture handler recorded, in the order the operation reaches it
#[derive(Debug)]
enum Seen {
    Environment {
        params: Params,
        headers: Vec<(String, String)>,
    },
    Get {
        name: String,
        api_base: String,
    },
    Delete {
        name: String,
        api_base: String,
    },
    Versions {
        name: String,
        api_base: String,
    },
}

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("the environment was refused")]
    Environment,
    #[error("{message}")]
    Refused { status: u16, message: String },
}

impl ProviderFailure for FixtureError {
    fn status_code(&self) -> u16 {
        match self {
            Self::Environment => 500,
            Self::Refused { status, .. } => *status,
        }
    }

    fn message(&self) -> String {
        self.to_string()
    }
}

// the Gemini agents config's shape, with every transform's output pinned by the test
struct Config {
    calls: Mutex<Vec<Seen>>,
    headers: RequestHeaders,
    refuse_environment: bool,
}

#[must_use]
fn fixture() -> Config {
    Config {
        calls: Mutex::new(Vec::new()),
        headers: RequestHeaders::new(),
        refuse_environment: false,
    }
}

impl Config {
    fn calls(&self) -> Vec<Seen> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, seen: Seen) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
    }
}

impl ErrorClasses for Config {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> FixtureError {
        FixtureError::Refused {
            status: status_code,
            message: message.to_owned(),
        }
    }
}

impl AgentsApiConfig for Config {
    type CreateOutput = AgentCreateResponse;
    type ListOutput = AgentCreateResponse;
    type GetOutput = AgentCreateResponse;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        litellm_params: &Params,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(Seen::Environment {
            params: litellm_params.clone(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        });
        if self.refuse_environment {
            return Err(FixtureError::Environment);
        }
        Ok(self.headers.clone())
    }

    fn get_complete_url(
        &self,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<String, FixtureError> {
        Ok(format!("{api_base}/v1beta/agents"))
    }

    fn transform_create_request(
        &self,
        _name: &str,
        _litellm_params: &Params,
    ) -> Result<Params, FixtureError> {
        Ok(Params::new())
    }

    fn transform_create_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, FixtureError> {
        agent_response(response, name)
    }

    fn transform_list_request(
        &self,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(PreparedRequest {
            url: format!("{api_base}/v1beta/agents"),
            params: Params::new(),
        })
    }

    fn transform_list_response(
        &self,
        _response: &UpstreamResponse,
    ) -> Result<AgentCreateResponse, FixtureError> {
        Ok(AgentCreateResponse::default())
    }

    fn transform_get_request(
        &self,
        name: &str,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<PreparedRequest, FixtureError> {
        self.record(Seen::Get {
            name: name.to_owned(),
            api_base: api_base.to_owned(),
        });
        Ok(PreparedRequest {
            url: format!("{api_base}/v1beta/agents/{name}"),
            params: Params::new(),
        })
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentCreateResponse, FixtureError> {
        agent_response(response, name)
    }
}

impl AgentsConfig for Config {
    type DeleteOutput = AgentDeleteResult;
    type VersionsOutput = AgentVersionsResponse;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(Seen::Environment {
            params: request.litellm_params.clone(),
            headers: secrets(request.headers),
        });
        if self.refuse_environment {
            return Err(FixtureError::Environment);
        }
        Ok(self.headers.clone())
    }

    fn transform_delete_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<String, FixtureError> {
        self.record(Seen::Delete {
            name: request.name.to_owned(),
            api_base: request.api_base.to_owned(),
        });
        Ok(format!("{}/agents/{}", request.api_base, request.name))
    }

    fn transform_delete_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<AgentDeleteResult, FixtureError> {
        ensure_success(response)?;
        Ok(AgentDeleteResult {
            name: name.to_owned(),
            deleted: Deleted(true),
            extra: BTreeMap::new(),
            hidden_params: BTreeMap::new(),
        })
    }

    fn transform_versions_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        self.record(Seen::Versions {
            name: request.name.to_owned(),
            api_base: request.api_base.to_owned(),
        });
        Ok(PreparedRequest {
            url: format!("{}/agents/{}/versions", request.api_base, request.name),
            params: page_params(request.litellm_params),
        })
    }

    fn transform_versions_response(
        &self,
        response: &UpstreamResponse,
        _name: &str,
    ) -> Result<AgentVersionsResponse, FixtureError> {
        ensure_success(response)?;
        let wire: VersionsWire = serde_json::from_slice(&response.body).unwrap_or_default();
        Ok(AgentVersionsResponse {
            agent_versions: wire.agent_versions,
            next_page_token: wire.next_page_token,
            extra: BTreeMap::new(),
            hidden_params: BTreeMap::new(),
        })
    }
}

#[derive(Default, serde::Deserialize)]
struct AgentWire {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, AgentJson>,
}

#[derive(Default, serde::Deserialize)]
struct VersionsWire {
    #[serde(default, rename = "agentVersions")]
    agent_versions: Vec<BTreeMap<String, AgentJson>>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

fn agent_response(
    response: &UpstreamResponse,
    name: &str,
) -> Result<AgentCreateResponse, FixtureError> {
    ensure_success(response)?;
    let wire: AgentWire = serde_json::from_slice(&response.body).unwrap_or_default();
    let id = wire.id.unwrap_or_else(|| name.to_owned());
    let surface = wire.name.unwrap_or_else(|| id.clone());
    Ok(AgentCreateResponse {
        id: Some(id),
        name: Some(surface),
        extra: wire.extra,
        hidden_params: BTreeMap::new(),
    })
}

fn ensure_success(response: &UpstreamResponse) -> Result<(), FixtureError> {
    if (200..300).contains(&response.status_code) {
        return Ok(());
    }
    Err(FixtureError::Refused {
        status: response.status_code,
        message: String::from_utf8(response.body.clone()).unwrap_or_default(),
    })
}

fn page_params(params: &Params) -> Params {
    let mut query = Params::new();
    if let Some(value) = params.get("page_size") {
        insert_query(&mut query, "pageSize", value);
    }
    if let Some(value) = params.get("page_token") {
        insert_query(&mut query, "pageToken", value);
    }
    query
}

fn insert_query(query: &mut Params, name: &str, value: &JsonValue) {
    match value {
        JsonValue::Text(text) => {
            query.insert(name.to_owned(), JsonValue::Text(text.clone()));
        }
        JsonValue::Number(number) => {
            query.insert(name.to_owned(), JsonValue::Text(number.to_string()));
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {}
    }
}

fn secrets(headers: &RequestHeaders) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn environment_params(config: &Config) -> Params {
    let calls = config.calls();
    let found = calls.iter().rev().find_map(|entry| match entry {
        Seen::Environment { params, .. } => Some(params.clone()),
        Seen::Get { .. } | Seen::Delete { .. } | Seen::Versions { .. } => None,
    });
    let Some(params) = found else {
        panic!("the environment is validated");
    };
    params
}

fn environment_headers(config: &Config) -> Vec<(String, String)> {
    let calls = config.calls();
    let found = calls.iter().rev().find_map(|entry| match entry {
        Seen::Environment { headers, .. } => Some(headers.clone()),
        Seen::Get { .. } | Seen::Delete { .. } | Seen::Versions { .. } => None,
    });
    let Some(headers) = found else {
        panic!("the environment is validated");
    };
    headers
}

struct Harness {
    clients: HttpClients,
    state: LoggingState,
    record: Logging,
    redactor: Redactor,
}

impl Harness {
    #[must_use]
    fn new() -> Self {
        let mut context = CorrelationContext::default();
        let redactor =
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles");
        let state = LoggingState::new(
            LoggingInit {
                model: String::new(),
                call_type: String::new(),
                start_time: moment(),
                litellm_call_id: "call-1".to_owned(),
                function_id: String::new(),
                messages: None,
                stream: None,
                litellm_trace_id: None,
                kwargs: IndexMap::new(),
                litellm_params: IndexMap::new(),
                callback_dynamic_params: IndexMap::new(),
                trusted_callback_vars: Vec::new(),
                standard_built_in_tools_params: IndexMap::new(),
                dynamic_callbacks: DynamicCallbacks::default(),
                applied_guardrails: None,
                request_logging: RequestLoggingFlags::default(),
                callback_policy: CallbackPolicy::default(),
            },
            CorrelationSetup {
                context: &mut context,
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated-trace"),
            || Duration::default(),
        )
        .expect("the logging fixture builds");
        Self {
            clients: HttpClients::new(policy(), proxy()),
            state,
            record: Logging::default(),
            redactor,
        }
    }

    fn deps(&mut self) -> AgentItemDeps<'_> {
        AgentItemDeps {
            clients: &mut self.clients,
            logging: &mut self.state,
            http: &mut self.record,
            redactor: &self.redactor,
        }
    }
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
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

fn proxy() -> Arc<ProxyPolicy> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap())
}

struct Call<'call> {
    name: &'call str,
    provider: Option<&'call str>,
    headers: BTreeMap<String, String>,
    kwargs: IndexMap<String, Json>,
}

impl<'call> Call<'call> {
    #[must_use]
    fn new(name: &'call str) -> Self {
        Self {
            name,
            provider: None,
            headers: BTreeMap::new(),
            kwargs: IndexMap::new(),
        }
    }

    #[must_use]
    fn provider(mut self, provider: &'call str) -> Self {
        self.provider = Some(provider);
        self
    }

    #[must_use]
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_owned(), value.to_owned());
        self
    }

    #[must_use]
    fn kwarg(mut self, name: &str, value: Json) -> Self {
        self.kwargs.insert(name.to_owned(), value);
        self
    }

    fn request(&self) -> AgentItemRequest<'_> {
        AgentItemRequest {
            name: self.name,
            custom_llm_provider: self.provider,
            extra_headers: self.headers.clone(),
            timeout: None,
            request_timeout: RequestTimeout {
                seconds: 6000.0,
                explicitly_set: false,
            },
            kwargs: &self.kwargs,
        }
    }
}

struct Serving {
    address: SocketAddr,
    request: JoinHandle<String>,
}

fn serve(answer: Vec<u8>) -> Serving {
    serve_on(TcpListener::bind("127.0.0.1:0").unwrap(), answer)
}

fn serve_on(listener: TcpListener, answer: Vec<u8>) -> Serving {
    let address = listener.local_addr().unwrap();
    let request = thread::Builder::new()
        .spawn(move || {
            let mut stream = accept(&listener);
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            let received = read_head(&mut stream).unwrap();
            stream.write_all(&answer).unwrap();
            received
        })
        .unwrap();
    Serving { address, request }
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _peer)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(
                    started.elapsed() < TIMEOUT,
                    "the client never connected to the fixture"
                );
                thread::park_timeout(RETRY);
            }
            Err(error) => panic!("the fixture listener failed: {error}"),
        }
    }
}

fn read_head<R>(stream: &mut R) -> IoResult<String>
where
    R: Read,
{
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        received.extend_from_slice(&byte);
    }
    String::from_utf8(received).map_err(IoError::other)
}

fn reply(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} fixture\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn head_of(serving: Serving) -> String {
    serving.request.join().unwrap()
}

fn request_line(head: &str) -> &str {
    head.lines().next().unwrap_or_default()
}

fn base(address: SocketAddr) -> String {
    format!("http://{address}")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn the_default_provider_is_gemini() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").kwarg("api_base", text(&base(serving.address)));
    let output = get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(output.id.as_deref(), Some("agent-1"));
    assert_eq!(harness.state.call_type, "get_agent");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    let params = environment_params(&config);
    assert_eq!(
        params.get("custom_llm_provider"),
        Some(&JsonValue::Text("gemini".to_owned()))
    );
    assert_eq!(
        request_line(&head_of(serving)),
        "GET /v1beta/agents/agent-1 HTTP/1.1"
    );
}

#[test]
fn a_kwargs_provider_reaches_the_agents_config() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").kwarg("custom_llm_provider", text("cohere"));
    let error = get(&config, harness.deps(), &call.request()).expect_err("cohere is unsupported");
    match error.class {
        ErrorClass::BadRequest(details) => {
            assert_eq!(details.model.as_deref(), Some(""));
            assert_eq!(details.provider.as_deref(), Some("cohere"));
        }
        other => panic!("expected a bad request, got {other:?}"),
    }
}

#[test]
fn a_null_kwargs_provider_stays_null_in_the_parameters() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("custom_llm_provider", Json::Null);
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    let params = environment_params(&config);
    assert_eq!(params.get("custom_llm_provider"), Some(&JsonValue::Null));
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    drop(head_of(serving));
}

#[test]
fn the_argument_provider_wins_over_kwargs() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .provider("gemini")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("custom_llm_provider", text("cohere"));
    get(&config, harness.deps(), &call.request()).expect("the argument provider is gemini");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    drop(head_of(serving));
}

#[test]
fn a_non_string_kwargs_provider_is_a_bad_request_naming_the_field() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").kwarg("custom_llm_provider", Json::Number(Number::from(5)));
    let error = get(&config, harness.deps(), &call.request()).expect_err("a number is a provider");
    match error.class {
        ErrorClass::BadRequest(details) => {
            assert_eq!(details.body_param.as_deref(), Some("custom_llm_provider"));
            assert_eq!(details.model.as_deref(), Some("agent-1"));
            assert_eq!(details.status.0, 400);
        }
        other => panic!("expected a bad request, got {other:?}"),
    }
}

#[test]
fn the_agent_name_is_the_only_optional_parameter() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-7").kwarg("api_base", text(&base(serving.address)));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(harness.state.model, "agent-7");
    assert_eq!(harness.state.optional_params.len(), 1);
    assert_eq!(
        harness.state.optional_params.get("name"),
        Some(&text("agent-7"))
    );
    assert_eq!(
        harness.state.model_call_details.get("name"),
        Some(&text("agent-7"))
    );
    drop(head_of(serving));
}

#[test]
fn delete_sends_a_delete_and_forwards_the_extra_headers() {
    let serving = serve(reply(200, "{}"));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .header("x-extra", "yes")
        .kwarg("api_base", text(&base(serving.address)));
    let output = delete(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(output.name, "agent-1");
    assert_eq!(harness.state.call_type, "delete_agent");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        harness.state.optional_params.get("name"),
        Some(&text("agent-1"))
    );
    assert_eq!(
        environment_headers(&config),
        vec![("x-extra".to_owned(), "yes".to_owned())]
    );
    assert_eq!(
        request_line(&head_of(serving)),
        "DELETE /v1beta/agents/agent-1 HTTP/1.1"
    );
}

#[test]
fn list_versions_forwards_the_pagination_parameters() {
    let body = r#"{"agentVersions": [{"name": "v1"}], "nextPageToken": "n"}"#;
    let serving = serve(reply(200, body));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("page_size", Json::Number(Number::from(2)))
        .kwarg("page_token", text("tok"));
    let output =
        list_versions(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(output.agent_versions.len(), 1);
    assert_eq!(output.next_page_token.as_deref(), Some("n"));
    assert_eq!(harness.state.call_type, "list_agent_versions");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        harness.state.optional_params.get("name"),
        Some(&text("agent-1"))
    );
    let params = environment_params(&config);
    assert_eq!(
        params.get("page_size"),
        Some(&JsonValue::Number(Number::from(2)))
    );
    assert_eq!(
        request_line(&head_of(serving)),
        "GET /v1beta/agents/agent-1/versions?pageSize=2&pageToken=tok HTTP/1.1"
    );
}

#[test]
fn get_pops_the_aget_agent_flag_before_the_parameters() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("aget_agent", Json::Bool(true));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    let params = environment_params(&config);
    assert!(!params.contains_key("aget_agent"));
    drop(head_of(serving));
}

#[test]
fn delete_pops_the_adelete_agent_flag_before_the_parameters() {
    let serving = serve(reply(200, "{}"));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("adelete_agent", Json::Bool(true));
    delete(&config, harness.deps(), &call.request()).expect("the fixture answers");
    let params = environment_params(&config);
    assert!(!params.contains_key("adelete_agent"));
    drop(head_of(serving));
}

#[test]
fn list_versions_pops_the_alist_agent_versions_flag_before_the_parameters() {
    let serving = serve(reply(200, r#"{"agentVersions": []}"#));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("alist_agent_versions", Json::Bool(true));
    list_versions(&config, harness.deps(), &call.request()).expect("the fixture answers");
    let params = environment_params(&config);
    assert!(!params.contains_key("alist_agent_versions"));
    drop(head_of(serving));
}

#[test]
fn logging_is_recorded_before_the_environment_fails() {
    let mut config = fixture();
    config.refuse_environment = true;
    let mut harness = Harness::new();
    let call = Call::new("agent-1");
    get(&config, harness.deps(), &call.request()).expect_err("the environment refuses");
    assert_eq!(harness.state.call_type, "get_agent");
    assert_eq!(
        harness.state.model_call_details.get("name"),
        Some(&text("agent-1"))
    );
    let calls = config.calls();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls.first(), Some(Seen::Environment { .. })));
}

#[test]
fn an_unsupported_provider_is_a_named_bad_request() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").provider("cohere");
    let error = get(&config, harness.deps(), &call.request()).expect_err("cohere is unsupported");
    match error.class {
        ErrorClass::BadRequest(details) => {
            assert_eq!(details.model.as_deref(), Some(""));
            assert_eq!(details.provider.as_deref(), Some("cohere"));
            assert!(
                details
                    .message
                    .contains("does not have a native agents API")
            );
        }
        other => panic!("expected a bad request, got {other:?}"),
    }
    assert_eq!(harness.state.call_type, "get_agent");
}

#[test]
fn a_missing_agent_is_a_not_found_error_with_the_agent_name() {
    let serving = serve(reply(404, r#"{"error": {"message": "missing"}}"#));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").kwarg("api_base", text(&base(serving.address)));
    let error = get(&config, harness.deps(), &call.request()).expect_err("the fixture refuses");
    match error.class {
        ErrorClass::NotFound(details) => {
            assert_eq!(details.status.0, 404);
            assert_eq!(details.model.as_deref(), Some("agent-1"));
            assert_eq!(details.provider.as_deref(), Some("gemini"));
        }
        other => panic!("expected a not found error, got {other:?}"),
    }
    drop(head_of(serving));
}

#[test]
fn a_failing_delete_is_an_internal_server_error_with_the_agent_name() {
    let serving = serve(reply(500, r#"{"error": {"message": "broken"}}"#));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").kwarg("api_base", text(&base(serving.address)));
    let error = delete(&config, harness.deps(), &call.request()).expect_err("the fixture refuses");
    match error.class {
        ErrorClass::InternalServer(context) => {
            assert_eq!(context.model.as_deref(), Some("agent-1"));
            assert_eq!(context.provider.as_deref(), Some("gemini"));
        }
        other => panic!("expected an internal server error, got {other:?}"),
    }
    drop(head_of(serving));
}

#[tokio::test]
async fn aget_sends_the_same_request_as_get() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").kwarg("api_base", text(&base(serving.address)));
    let output = aget(&config, harness.deps(), &call.request())
        .await
        .expect("the fixture answers");
    assert_eq!(output.id.as_deref(), Some("agent-1"));
    assert_eq!(harness.state.call_type, "get_agent");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        request_line(&head_of(serving)),
        "GET /v1beta/agents/agent-1 HTTP/1.1"
    );
}

#[tokio::test]
async fn adelete_sends_the_same_request_as_delete() {
    let serving = serve(reply(200, "{}"));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1").kwarg("api_base", text(&base(serving.address)));
    let output = adelete(&config, harness.deps(), &call.request())
        .await
        .expect("the fixture answers");
    assert!(output.deleted.0);
    assert_eq!(harness.state.call_type, "delete_agent");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        request_line(&head_of(serving)),
        "DELETE /v1beta/agents/agent-1 HTTP/1.1"
    );
}

#[tokio::test]
async fn alist_versions_sends_the_same_request_as_list_versions() {
    let serving = serve(reply(200, r#"{"agentVersions": []}"#));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("agent-1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("page_size", Json::Number(Number::from(3)));
    let output = alist_versions(&config, harness.deps(), &call.request())
        .await
        .expect("the fixture answers");
    assert!(output.agent_versions.is_empty());
    assert_eq!(harness.state.call_type, "list_agent_versions");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        request_line(&head_of(serving)),
        "GET /v1beta/agents/agent-1/versions?pageSize=3 HTTP/1.1"
    );
}
