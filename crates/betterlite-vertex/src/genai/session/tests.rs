#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_bedrock::aws_settings::sources::{Credentials, SourceError};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::studio_identity::Raw as StudioRaw;
use betterlite_config::sections::vertex_credential_sources::Raw as VertexRaw;
use betterlite_config::sections::vertex_schema_shaping::Raw as SchemaRaw;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::genai::chat_call::{
    CompletionCall, CompletionExecutor, CompletionOutput,
};
use betterlite_core::api::genai::generate_content::GenerateContentAnswer;
use betterlite_core::api::genai_http::config::{AuthCall, GenerateContentHttpConfig as _};
use betterlite_core::api::genai_http::service::ClientSource as _;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::genai_setup::config::GenerateContentProviderConfig as _;
use betterlite_core::genai_setup::params::GenerateContentLiteLLMParams;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_logging::logging_pre_post::env::TracingSink;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::BundledCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use http::Method;
use http::header::HeaderMap;
use indexmap::IndexMap;
use url::Url;

use super::call::GenaiCall;
use super::clients::NativeClients;
use super::provider::gemini_key;
use super::{Entry, GenaiSession, SessionParts, logging_env};
use crate::aws_federation::AwsCredentialsResolver;
use crate::credential_sources::sdk::SystemClock;
use crate::google_native_content::capabilities::NativeProvider;
use crate::studio_identity::AuthValue;

#[test]
fn gemini_key_reads_the_kwarg() {
    let extra = Object::from([("gemini_api_key".to_owned(), Json::Text("probe".to_owned()))]);
    let key = gemini_key(&extra).expect("the kwarg is a studio key");
    assert!(matches!(key, AuthValue::Key(_)));
    assert!(gemini_key(&Object::new()).is_none());
}

#[test]
fn call_types_are_the_litellm_entry_names() {
    assert_eq!(Entry::Blocking.call_type(), "generate_content");
    assert_eq!(Entry::Await.call_type(), "agenerate_content");
    assert_eq!(Entry::BlockingStream.call_type(), "generate_content_stream");
    assert_eq!(Entry::AwaitStream.call_type(), "agenerate_content_stream");
}

#[test]
fn optional_params_follow_the_provider() {
    let config = Object::from([
        (
            "top_p".to_owned(),
            Json::Number(serde_json::Number::from(1_i64)),
        ),
        ("mystery".to_owned(), Json::Bool(true)),
    ]);
    let session = session();
    let studio = session
        .config(NativeProvider::Studio)
        .map_optional_params(&config, "gemini-2.5-flash");
    let vertex = session
        .config(NativeProvider::Vertex)
        .map_optional_params(&config, "gemini-2.5-flash");
    assert_eq!(studio.get("topP"), config.get("top_p"));
    assert!(studio.get("top_p").is_none());
    assert!(studio.get("mystery").is_none());
    assert_eq!(vertex.get("top_p"), config.get("top_p"));
    assert_eq!(vertex.get("mystery"), config.get("mystery"));
}

#[test]
fn top_level_fields_are_the_generate_content_config_fields() {
    assert_eq!(
        session()
            .config(NativeProvider::Studio)
            .request_top_level_fields(),
        &["safetySettings", "toolConfig", "cachedContent", "labels"][..]
    );
}

#[test]
fn decode_response_reads_the_candidates_and_names_a_bad_body() {
    let config = session().config(NativeProvider::Studio);
    let answered = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: br#"{"candidates":[{"content":{"role":"model","parts":[{"text":"hello"}]}}]}"#
            .to_vec(),
    };
    let decoded = config.decode_response(&answered).unwrap();
    assert_eq!(decoded.candidates().unwrap().len(), 1);
    let refused = UpstreamResponse {
        status: 502,
        headers: HeaderMap::new(),
        body: b"<html>bad gateway</html>".to_vec(),
    };
    let error = config
        .decode_response(&refused)
        .expect_err("a body that is not JSON has no candidates");
    assert_eq!(error.status_code, 502);
    assert!(
        error
            .message
            .starts_with("Error transforming generate content response")
    );
}

#[tokio::test]
async fn studio_auth_sends_the_key_to_generative_language() {
    let params = GenerateContentLiteLLMParams::from_kwargs(&Object::from([(
        "api_key".to_owned(),
        Json::Text("probe-key".to_owned()),
    )]))
    .unwrap();
    let call = AuthCall {
        api_base: None,
        model: "gemini-2.5-flash",
        params: &params,
        stream: false,
    };
    let config = session().config(NativeProvider::Studio);
    let target = config.auth_token_and_url(&call).await.unwrap();
    assert_eq!(
        target.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(
        target.headers.get("x-goog-api-key").map(String::as_str),
        Some("probe-key")
    );
    assert_eq!(
        target.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[tokio::test]
async fn clients_keep_the_default_for_true_and_none() {
    let clients = clients();
    let absent = request_user_agent(&clients.client(None).unwrap()).await;
    let enabled = request_user_agent(&clients.client(Some(&Json::Bool(true))).unwrap()).await;
    assert_eq!(absent, "default-agent");
    assert_eq!(enabled, "default-agent");
}

#[tokio::test]
async fn clients_build_a_new_client_for_false_and_a_path() {
    let clients = clients();
    let disabled = request_user_agent(&clients.client(Some(&Json::Bool(false))).unwrap()).await;
    let path = ca_path();
    let ca_file = request_user_agent(&clients.client(Some(&Json::Text(path))).unwrap()).await;
    assert_eq!(disabled, "probe/1");
    assert_eq!(ca_file, "probe/1");
}

#[test]
fn request_reads_every_field() {
    let call: GenaiCall = serde_json::from_str(
        r#"{
            "model": "gemini/gemini-2.5-flash",
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "config": {"temperature": 0.5},
            "tools": [{"functionDeclarations": []}],
            "extra_headers": {"x-probe": "one"},
            "extra_query": {"probe": "two"},
            "extra_body": {"probe": "three"},
            "timeout": 2.5,
            "custom_llm_provider": "vertex_ai",
            "kwargs": {"mock_response": "probe"}
        }"#,
    )
    .unwrap();
    let request = call.request(call.timeout().unwrap());
    assert_eq!(request.model, call.model.as_str());
    assert_eq!(request.contents, &call.contents);
    assert_eq!(request.config, call.config.as_ref());
    assert_eq!(request.tools, call.tools.as_ref());
    assert_eq!(request.extra_headers, call.extra_headers.as_ref());
    assert_eq!(request.extra_query, call.extra_query.as_ref());
    assert_eq!(request.extra_body, call.extra_body.as_ref());
    assert_eq!(request.timeout, Some(Duration::from_millis(2500)));
    assert_eq!(request.custom_llm_provider, Some("vertex_ai"));
    assert_eq!(request.kwargs, &call.kwargs);
}

#[test]
fn timeout_refuses_a_negative_number() {
    let call: GenaiCall = serde_json::from_str(
        r#"{"model": "gemini/gemini-2.5-flash", "contents": [], "timeout": -1.0}"#,
    )
    .unwrap();
    call.timeout()
        .expect_err("a negative number of seconds is not a duration");
}

#[tokio::test]
async fn run_answers_the_mock_without_a_provider() {
    let session = session();
    let call: GenaiCall = serde_json::from_str(
        r#"{"model": "gemini/gemini-2.5-flash", "contents": [], "kwargs": {"mock_response": "probe"}}"#,
    )
    .unwrap();
    let request = call.request(call.timeout().unwrap());
    let mut state = logging_state(&session.redactor);
    let sink = TracingSink;
    let env = logging_env(&session.redactor, &sink);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let executor = CountingExecutor::default();
    let answer = session
        .run(&request, &executor, &mut log, Entry::Blocking)
        .await
        .unwrap();
    assert_eq!(executor.calls.load(Ordering::Relaxed), 0);
    let GenerateContentAnswer::Mock(json) = answer else {
        panic!("a string mock is the whole answer");
    };
    assert_eq!(first_candidate_text(&json), "probe");
}

// the AWS source the server has today: one Vertex workload identity pool source less than the SDK, so its load fails
struct UnavailableAws;

impl AwsCredentialsResolver for UnavailableAws {
    fn credentials(
        &self,
        _settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Err(CredentialError::Source(SourceError::MissingProvider)))
    }
}

// a chat adapter that records every call, so a mock that reached it would answer an error and a count
#[derive(Default)]
struct CountingExecutor {
    calls: AtomicUsize,
}

#[derive(Debug, thiserror::Error)]
#[error("the mock path never calls the chat adapter")]
struct NeverCalled;

impl CompletionExecutor for CountingExecutor {
    type Error = NeverCalled;

    fn completion(&self, _call: CompletionCall) -> Result<CompletionOutput, NeverCalled> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Err(NeverCalled)
    }

    fn acompletion(
        &self,
        _call: CompletionCall,
    ) -> impl Future<Output = Result<CompletionOutput, NeverCalled>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        ready(Err(NeverCalled))
    }
}

fn session() -> GenaiSession {
    let environment = BTreeMap::new();
    let studio = StudioRaw::default().convert(&environment).unwrap();
    let vertex_settings = VertexRaw::default().convert(&environment).unwrap();
    let schema = SchemaRaw::default().convert(&environment).unwrap();
    let request_timeout = RequestTimeout::resolve(&environment).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    GenaiSession::new(SessionParts {
        environment,
        studio,
        vertex: vertex_settings,
        schema,
        request_timeout,
        catalog: BundledCatalog::default().get(),
        clients: clients(),
        clock: Arc::new(SystemClock),
        redactor,
        aws: Arc::new(UnavailableAws),
    })
}

// the default client is built from a policy of its own, so a rebuilt client's agent tells the two apart
fn clients() -> NativeClients {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let tls = TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "default-agent".to_owned(),
    };
    let default = ProxiedClient::outbound_with_tls(&policy, &tls).unwrap();
    NativeClients::new(default, "probe/1".to_owned())
}

fn ca_path() -> String {
    format!(
        "{}/src/genai/session/fixtures/ca.pem",
        env!("CARGO_MANIFEST_DIR")
    )
}

// the User-Agent header the client puts on the wire, read by the fixture server that answers one request
async fn request_user_agent(client: &ProxiedClient) -> String {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let request = OutboundRequest {
        method: Method::POST,
        url: Url::parse(&format!("http://{}/probe", serving.address)).unwrap(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(b"{}".to_vec()),
        deadline: Duration::from_secs(5),
    };
    client.send_buffered(request).await.unwrap();
    let recorded = serving.finish().unwrap();
    recorded
        .head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _value)| name.eq_ignore_ascii_case("user-agent"))
        .map(|(_name, value)| value.trim().to_owned())
        .unwrap()
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let mut correlation = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "gemini/gemini-2.5-flash".to_owned(),
            call_type: Entry::Blocking.call_type().to_owned(),
            start_time: Utc::now().fixed_offset(),
            litellm_call_id: "call-id".to_owned(),
            function_id: "function-id".to_owned(),
            messages: None,
            stream: Some(false),
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
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "trace-id".to_owned(),
        || Duration::ZERO,
    )
    .unwrap()
}

fn first_candidate_text(answer: &Json) -> &str {
    let Json::Object(fields) = answer else {
        panic!("the mock answer is an object");
    };
    let Some(Json::List(candidates)) = fields.get("candidates") else {
        panic!("the mock answer carries candidates");
    };
    let Some(Json::Object(candidate)) = candidates.first() else {
        panic!("the first candidate is an object");
    };
    let Some(Json::Object(content)) = candidate.get("content") else {
        panic!("the first candidate carries content");
    };
    let Some(Json::List(parts)) = content.get("parts") else {
        panic!("the content carries parts");
    };
    let Some(Json::Object(part)) = parts.first() else {
        panic!("the first part is an object");
    };
    let Some(Json::Text(text)) = part.get("text") else {
        panic!("the first part carries the text");
    };
    text
}
