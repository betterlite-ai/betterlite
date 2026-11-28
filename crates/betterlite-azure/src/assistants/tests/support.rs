#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::{RunEvent, RunReply, RunRequest};
use betterlite_openai::thread_runs::stream::{
    RunChunks, RunEventHandler, RunStream, RunStreamTransport,
};
use futures_util::stream::iter;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::json;

use crate::assistants::error::AssistantsRunError;
use crate::assistants::operations::{AssistantsRunsApi, RunCall, RunDispatch, RunInvocation};
use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_assistant_definitions::operations::ClientRequest;
use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_urls::AuthParams;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::projection::{CallableProvider, LiteLlmParams, SessionKind};
use crate::oidc_exchange::OidcExchangeInput;

pub(super) const KEY: &str = "sk-fixture";
pub(super) const BASE: &str = "https://example.test";
pub(super) const VERSION: &str = "2024-02-01";
pub(super) const CREATE_URL: &str =
    "https://example.test/openai/threads/thread_1/runs?api-version=2024-02-01";
pub(super) const POLL_URL: &str =
    "https://example.test/openai/threads/thread_1/runs/run_1?api-version=2024-02-01";
pub(super) const ERROR_BODY: &str =
    r#"{"error":{"message":"nope","type":"invalid_request_error"}}"#;
pub(super) const CREATED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_1","status":"queued","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[{"type":"code_interpreter"}]}"#;
pub(super) const COMPLETED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"completed_at":1700000005,"assistant_id":"asst_1","thread_id":"thread_1","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"metadata":{"tag":"x"},"tools":[{"type":"code_interpreter"}],"usage":{"completion_tokens":10,"prompt_tokens":5,"total_tokens":15},"temperature":0.5,"top_p":0.25}"#;
pub(super) const REQUIRES_ACTION: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_1","status":"requires_action","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"required_action":{"type":"submit_tool_outputs","submit_tool_outputs":{"tool_calls":[]}},"tools":[{"type":"code_interpreter"}]}"#;
pub(super) const FAILED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"failed_at":1700000003,"assistant_id":"asst_1","thread_id":"thread_1","status":"failed","instructions":"base","last_error":{"code":"server_error","message":"boom"},"model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[{"type":"code_interpreter"}]}"#;
pub(super) const STREAM_BODY: &str = concat!(
    "event: thread.run.created\n",
    "data: {\"id\":\"run_1\",\"object\":\"thread.run\",\"created_at\":1700000000,\"assistant_id\":\"asst_1\",\"thread_id\":\"thread_1\",\"status\":\"queued\",\"instructions\":\"base\",\"model\":\"gpt-4o-mini\",\"parallel_tool_calls\":true,\"tools\":[{\"type\":\"code_interpreter\"}]}\n",
    "\n",
    "event: thread.run.completed\n",
    "data: {\"id\":\"run_1\",\"object\":\"thread.run\",\"created_at\":1700000000,\"completed_at\":1700000005,\"assistant_id\":\"asst_1\",\"thread_id\":\"thread_1\",\"status\":\"completed\",\"instructions\":\"base\",\"model\":\"gpt-4o-mini\",\"parallel_tool_calls\":true,\"tools\":[{\"type\":\"code_interpreter\"}]}\n",
    "\n",
    "data: [DONE]\n",
    "\n",
);

pub(super) type Headers = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) deadline: Duration,
    pub(super) headers: Headers,
    pub(super) body: Option<JsonValue>,
}

type Queued = Result<(u16, HeaderMap, Vec<u8>), TransportError>;

#[derive(Default)]
pub(super) struct Recorder {
    replies: Mutex<VecDeque<Queued>>,
    seen: Mutex<Vec<Seen>>,
}

impl Recorder {
    pub(super) fn start() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(super) fn queue(&self, status: u16, body: &str) {
        self.queue_with(status, HeaderMap::new(), body);
    }

    // A zero poll interval keeps the fake provider's wait out of the test's wall clock.
    pub(super) fn queue_poll(&self, status: u16, body: &str) {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("openai-poll-after-ms"),
            HeaderValue::from_static("0"),
        );
        self.queue_with(status, headers, body);
    }

    pub(super) fn queue_with(&self, status: u16, headers: HeaderMap, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back(Ok((status, headers, body.as_bytes().to_vec())));
    }

    pub(super) fn fail_next(&self) {
        self.replies
            .lock()
            .unwrap()
            .push_back(Err(TransportError::Closed));
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn record(&self, request: &OutboundRequest) {
        self.seen.lock().unwrap().push(recorded(request));
    }

    fn next(&self) -> Queued {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Err(TransportError::Closed))
    }

    fn reply(&self, request: &OutboundRequest) -> Queued {
        self.record(request);
        self.next()
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn recorded(request: &OutboundRequest) -> Seen {
    let mut url = request.url.clone();
    merge_query(&mut url, &request.query);
    let body = match &request.body {
        RequestBody::Json(bytes) => Some(serde_json::from_slice::<JsonValue>(bytes).unwrap()),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
    };
    let mut headers = Headers::new();
    for name in [
        "accept",
        "api-key",
        "authorization",
        "content-type",
        "openai-beta",
    ] {
        if let Some(value) = header(&request.headers, name) {
            headers.insert(name.to_owned(), value);
        }
    }
    // The shared transport adds this header to a JSON body itself.
    if body.is_some() {
        headers
            .entry(String::from("content-type"))
            .or_insert_with(|| String::from("application/json"));
    }
    Seen {
        method: request.method.as_str().to_owned(),
        url: url.as_str().to_owned(),
        deadline: request.deadline,
        headers,
        body,
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let reply = self.reply(&request);
        ready(reply.map(|(status, headers, body)| UpstreamResponse {
            status,
            headers,
            body,
        }))
    }
}

impl RunStreamTransport for Recorder {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        let reply = self.reply(&request);
        match reply {
            Ok((status, _headers, body)) if status >= 400 => {
                ready(Err(ThreadRunsError::Status(status, body)))
            }
            Ok((_status, _headers, body)) => {
                let chunks: RunChunks = Box::pin(iter(vec![Ok(body)]));
                ready(Ok(chunks))
            }
            Err(error) => ready(Err(ThreadRunsError::Transport(error))),
        }
    }
}

pub(super) fn api(transport: &Arc<Recorder>) -> AssistantsRunsApi<Recorder> {
    AssistantsRunsApi {
        transport: Arc::clone(transport),
    }
}

// One facade call as a caller makes it, with the fake provider standing in for Azure.
pub(super) async fn invoked(
    transport: &Arc<Recorder>,
    fixture: &Fixture,
    run: &RunRequest,
    dispatch: RunDispatch,
    handler: Option<Arc<dyn RunEventHandler>>,
) -> RunInvocation<Recorder> {
    let call = RunCall {
        dispatch,
        run,
        handler,
    };
    let request = fixture.request();
    let api = api(transport);
    api.run(&request, None, call).await.unwrap()
}

impl RunInvocation<Recorder> {
    // A completed run is the only outcome a poll test accepts.
    fn polled(self) -> (ClientKind, RunReply) {
        match self {
            Self::Polled(kind, reply) => (kind, *reply),
            Self::Streaming(_) => panic!("expected a polled run"),
        }
    }
}

pub(super) async fn polled(
    transport: &Arc<Recorder>,
    fixture: &Fixture,
    run: &RunRequest,
) -> (ClientKind, RunReply) {
    invoked(transport, fixture, run, RunDispatch::default(), None)
        .await
        .polled()
}

pub(super) async fn streamed(
    transport: &Arc<Recorder>,
    fixture: &Fixture,
    run: &RunRequest,
    dispatch: RunDispatch,
    handler: Option<Arc<dyn RunEventHandler>>,
) -> RunStream<Recorder> {
    match invoked(transport, fixture, run, dispatch, handler).await {
        RunInvocation::Streaming(stream) => *stream,
        RunInvocation::Polled(_kind, reply) => panic!("expected a stream, got {}", reply.id),
    }
}

pub(super) async fn failed(
    transport: &Arc<Recorder>,
    fixture: &Fixture,
    run: &RunRequest,
) -> AssistantsRunError {
    let call = RunCall {
        dispatch: RunDispatch::default(),
        run,
        handler: None,
    };
    let request = fixture.request();
    let outcome = api(transport).run(&request, None, call).await;
    match outcome {
        Err(error) => error,
        Ok(_) => panic!("expected a failed run"),
    }
}

// The run the tests call: the create body's keywords plus the thread id the path carries.
pub(super) fn run_request() -> RunRequest {
    let mut body = run_body(false);
    body["thread_id"] = json!("thread_1");
    body.as_object_mut()
        .expect("a body")
        .remove("stream")
        .expect("a stream flag");
    serde_json::from_value(body).expect("a run request")
}

// The body one run request carries, as JSON text, so a test states the wire form it expects.
pub(super) fn run_body(stream: bool) -> JsonValue {
    let text = format!(
        concat!(
            r#"{{"assistant_id":"asst_1","additional_instructions":"extra","instructions":"base","#,
            r#""metadata":{{"tag":"x"}},"model":"gpt-4o-mini","stream":{stream},"#,
            r#""tools":[{{"type":"code_interpreter"}}]}}"#,
        ),
        stream = stream
    );
    serde_json::from_str(&text).unwrap()
}

pub(super) fn parsed(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(super) fn status_of(error: AssistantsRunError) -> (u16, Vec<u8>) {
    match error {
        AssistantsRunError::Runs(ThreadRunsError::Status(status, body)) => (status, body),
        other @ (AssistantsRunError::Client(_)
        | AssistantsRunError::Runs(_)
        | AssistantsRunError::Encode(_)) => panic!("expected a status failure, got {other:?}"),
    }
}

pub(super) struct FixedToken(pub(super) &'static str);

impl CallableTokenProvider for FixedToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from(self.0)))
    }
}

pub(super) struct EmptyToken;

impl CallableTokenProvider for EmptyToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from("")))
    }
}

pub(super) struct FailingToken;

impl CallableTokenProvider for FailingToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Err(ProviderFailure {
            class: String::from("ClientAuthenticationError"),
            message: String::from("the provider rejected the request"),
        })
    }
}

pub(super) struct NonTextToken;

impl CallableTokenProvider for NonTextToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::NonText {
            type_name: String::from("int"),
        })
    }
}

pub(super) fn provider(token: Arc<dyn CallableTokenProvider>) -> CallableProvider {
    CallableProvider {
        provider: token,
        identity: 1,
    }
}

// A caller that holds no Azure AD token source: every token request refuses.
pub(super) struct NoSources;

fn refuse<T>(kind: &str) -> Result<T, SourceFailure> {
    Err(SourceFailure {
        class: String::from("CredentialUnavailableError"),
        status: None,
        message: format!("no {kind} source is configured"),
    })
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("client secret")
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("username and password")
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("SDK refresh")
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        refuse("OIDC")
    }
}

pub(super) struct Fixture {
    pub(super) settings: Settings,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sources: NoSources,
    pub(super) params: LiteLlmParams,
    pub(super) provider: Option<CallableProvider>,
    pub(super) api_key: Option<SecretString>,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) azure_ad_token: Option<SecretString>,
    pub(super) timeout: f64,
    pub(super) max_retries: Option<i64>,
    pub(super) refresh_enabled: bool,
    pub(super) session: SessionKind,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            settings: Settings {
                default_max_retries: 2,
                azure_default_api_version: String::from("2025-02-01-preview"),
            },
            environment: BTreeMap::new(),
            sources: NoSources,
            params: LiteLlmParams {
                azure_ad_token: None,
                auth: AuthParams::default(),
                max_retries: None,
                timeout: None,
            },
            provider: None,
            api_key: Some(SecretString::from(KEY)),
            api_base: Some(String::from(BASE)),
            api_version: Some(String::from(VERSION)),
            azure_ad_token: None,
            timeout: 600.0,
            max_retries: None,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
        }
    }
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn request(&self) -> ClientRequest<'_> {
        ClientRequest {
            params: self.params.clone(),
            provider: self.provider.clone(),
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            api_version: self.api_version.clone(),
            azure_ad_token: self.azure_ad_token.clone(),
            timeout: self.timeout,
            max_retries: self.max_retries,
            refresh_enabled: self.refresh_enabled,
            session: self.session,
            settings: &self.settings,
            environment: &self.environment,
            sources: &self.sources,
        }
    }
}

pub(super) fn supplied_client(
    kind: ClientKind,
    transport: &Arc<Recorder>,
) -> AzureAssistantsClient<'static, Recorder> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport: Arc::clone(transport),
        base_url: String::from("https://supplied.test/openai"),
        api_version: String::from("2024-01-01"),
        key: Some(SecretString::from("supplied-key")),
        token: None,
        deadline: Duration::from_secs(42),
    }
}

// The SDK reports a timeout first, then the exception, and always ends the handler after a failure.
pub(super) struct RecordingHandler {
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingHandler {
    pub(super) fn start() -> Arc<Self> {
        Arc::new(Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn log(&self, entry: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

impl RunEventHandler for RecordingHandler {
    fn on_event(&self, event: &RunEvent) {
        self.log(event.name().to_owned());
    }

    fn on_exception(&self, error: &ThreadRunsError) {
        self.log(format!("exception:{}", error_kind(error)));
    }

    fn on_timeout(&self) {
        self.log(String::from("timeout"));
    }

    fn on_end(&self) {
        self.log(String::from("end"));
    }
}

fn error_kind(error: &ThreadRunsError) -> &'static str {
    if matches!(error, ThreadRunsError::ProviderError { .. }) {
        "stream_error"
    } else {
        "other"
    }
}

pub(super) fn definitions(error: AssistantsRunError) -> DefinitionsError {
    match error {
        AssistantsRunError::Client(failure) => failure,
        other @ (AssistantsRunError::Runs(_) | AssistantsRunError::Encode(_)) => {
            panic!("expected a client failure, got {other:?}")
        }
    }
}
