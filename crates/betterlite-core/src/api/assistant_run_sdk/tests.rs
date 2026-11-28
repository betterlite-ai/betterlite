#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::mem::take;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::{RunEvent, RunReply};
use betterlite_openai::thread_runs::stream::{
    RunChunks, RunEventHandler, RunStream, RunStreamTransport,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::stream::iter;
use futures_util::{Stream, StreamExt as _};
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::mpsc;

use crate::api::assistant_run_sdk::call::{
    RunClient, RunClientClass, RunFlag, RunParams, RunThreadCall, RunTimeout,
};
use crate::api::assistant_run_sdk::error::{AssistantRunError, RunDispatchError as _, RunFailure};
use crate::api::assistant_run_sdk::provider::ProviderRuns;
use crate::api::assistant_run_sdk::service::{
    AssistantRunSdk, RunOperations, RunOutcome, RunTarget,
};
use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::api::resource_sdk_settings::timeout::{StructuredTimeout, TimeoutValue};

const BASE: &str = "https://example.test/v1";
const QUEUED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_a","thread_id":"thread_a","status":"queued","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[{"type":"code_interpreter"}]}"#;
const COMPLETED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"completed_at":1700000005,"assistant_id":"asst_a","thread_id":"thread_a","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"metadata":{"tag":"x"},"tools":[{"type":"code_interpreter"}]}"#;
const REQUIRES_ACTION: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_a","thread_id":"thread_a","status":"requires_action","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"required_action":{"submit_tool_outputs":{"tool_calls":[{"id":"call_1","function":{"arguments":"{}","name":"lookup"},"type":"function"}]},"type":"submit_tool_outputs"},"tools":[{"type":"code_interpreter"}]}"#;
const FAILED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"failed_at":1700000005,"assistant_id":"asst_a","thread_id":"thread_a","status":"failed","instructions":"base","last_error":{"code":"server_error","message":"boom"},"model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[{"type":"code_interpreter"}]}"#;
const NOT_FOUND: &str = r#"{"error":{"message":"no such run","type":"not_found_error","param":"thread_id","code":"run_missing"}}"#;
const SERVER_ERROR: &str =
    r#"{"error":{"message":"oops","type":"server_error","param":"model","code":"server_boom"}}"#;

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    url: String,
    deadline: Duration,
    authorization: Option<String>,
    api_key: Option<String>,
    organization: Option<String>,
    beta: Option<String>,
    body: Option<JsonValue>,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, Vec<u8>)>>,
    gate: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    prepared: Mutex<Option<RunChunks>>,
}

struct GateChunks {
    receiver: mpsc::Receiver<Vec<u8>>,
}

impl Stream for GateChunks {
    type Item = Result<Vec<u8>, ThreadRunsError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut()
            .receiver
            .poll_recv(cx)
            .map(|chunk| chunk.map(Ok))
    }
}

impl Recorder {
    fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back((status, body.as_bytes().to_vec()));
    }

    fn queue_twice(&self, body: &str) {
        self.queue(200, body);
        self.queue(200, body);
    }

    fn gate(&self) -> mpsc::Sender<Vec<u8>> {
        let (sender, receiver) = mpsc::channel(1);
        *self.gate.lock().unwrap() = Some(receiver);
        sender
    }

    fn prepare(&self, chunks: Vec<Vec<u8>>) {
        let frames = chunks.into_iter().map(Ok).collect::<Vec<_>>();
        *self.prepared.lock().unwrap() = Some(Box::pin(iter(frames)));
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn record(&self, request: &OutboundRequest) {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: url.as_str().to_owned(),
            deadline: request.deadline,
            authorization: header_of(&request.headers, "authorization"),
            api_key: header_of(&request.headers, "api-key"),
            organization: header_of(&request.headers, "openai-organization"),
            beta: header_of(&request.headers, "openai-beta"),
            body: match &request.body {
                RequestBody::Json(bytes) => Some(serde_json::from_slice(bytes).unwrap()),
                RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                    None
                }
            },
        });
    }

    fn next_reply(&self) -> (u16, Vec<u8>) {
        self.replies.lock().unwrap().pop_front().unwrap_or_else(|| {
            (
                500,
                b"{\"error\":{\"message\":\"no reply queued\"}}".to_vec(),
            )
        })
    }
}

fn header_of(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        let (status, body) = self.next_reply();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body,
        }))
    }
}

impl RunStreamTransport for Recorder {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        self.record(&request);
        let prepared = self.prepared.lock().unwrap().take();
        if let Some(chunks) = prepared {
            return ready(Ok(chunks));
        }
        let gated = self.gate.lock().unwrap().take();
        if let Some(receiver) = gated {
            let chunks: RunChunks = Box::pin(GateChunks { receiver });
            return ready(Ok(chunks));
        }
        let (status, body) = self.next_reply();
        if status >= 400 {
            return ready(Err(ThreadRunsError::Status(status, body)));
        }
        let chunks: RunChunks = Box::pin(iter(vec![Ok(body)]));
        ready(Ok(chunks))
    }
}

#[derive(Default)]
struct RecordingHandler {
    events: Mutex<Vec<String>>,
    ended: AtomicBool,
}

impl RecordingHandler {
    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }
}

impl RunEventHandler for RecordingHandler {
    fn on_event(&self, event: &RunEvent) {
        self.events.lock().unwrap().push(event.name().to_owned());
    }

    fn on_exception(&self, _error: &ThreadRunsError) {}

    fn on_timeout(&self) {}

    fn on_end(&self) {
        self.ended.store(true, Ordering::Release);
    }
}

struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}

// the wrapper's whole job is the target it hands RunOperations, so a recording seam observes it
enum CapturedTarget {
    OpenAi {
        kind: ClientKind,
        handler: bool,
        options: ClientOptions,
        supplied: Option<RunClient>,
    },
    Azure {
        params: LiteLlmParams,
        api_key: Option<SecretString>,
        api_base: Option<String>,
        api_version: Option<String>,
        azure_ad_token: Option<SecretString>,
        max_retries: Option<i64>,
    },
}

#[derive(Default)]
struct CaptureRuns {
    targets: Mutex<Vec<CapturedTarget>>,
    reply: Mutex<Option<RunReply>>,
}

impl CaptureRuns {
    fn capture(&self, target: RunTarget<'_>) {
        let captured = match target {
            RunTarget::OpenAi(run) => {
                let inner = *run;
                CapturedTarget::OpenAi {
                    kind: inner.kind,
                    handler: inner.handler.is_some(),
                    options: inner.options,
                    supplied: inner.supplied,
                }
            }
            RunTarget::Azure(run) => {
                let inner = *run;
                CapturedTarget::Azure {
                    params: inner.client.params,
                    api_key: inner.client.api_key,
                    api_base: inner.client.api_base,
                    api_version: inner.client.api_version,
                    azure_ad_token: inner.client.azure_ad_token,
                    max_retries: inner.client.max_retries,
                }
            }
        };
        self.targets.lock().unwrap().push(captured);
    }

    fn targets(&self) -> Vec<CapturedTarget> {
        take(&mut *self.targets.lock().unwrap())
    }
}

impl RunOperations for CaptureRuns {
    type Stream = ();

    fn poll(
        &self,
        target: RunTarget<'_>,
    ) -> impl Future<Output = Result<Option<RunReply>, AssistantsRunError>> + Send {
        self.capture(target);
        let reply = self.reply.lock().unwrap().clone();
        ready(Ok(reply))
    }

    fn stream(&self, target: RunTarget<'_>) -> Result<(), AssistantsRunError> {
        self.capture(target);
        Ok(())
    }
}

struct Fixture {
    settings: ResourceSdkSettings,
    azure: AzureSettings,
    environment: BTreeMap<String, String>,
    sources: NoSources,
    redactor: Redactor,
}

impl Fixture {
    fn new() -> Self {
        let mut settings = empty_settings();
        settings.api_key = Some(secret("sk-test"));
        Self {
            settings,
            azure: AzureSettings::default(),
            environment: BTreeMap::new(),
            sources: NoSources,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
        }
    }

    fn sdk<O>(&self, operations: O) -> AssistantRunSdk<'_, O> {
        AssistantRunSdk {
            operations,
            settings: &self.settings,
            azure: &self.azure,
            environment: &self.environment,
            sources: &self.sources,
            redactor: &self.redactor,
        }
    }
}

fn empty_settings() -> ResourceSdkSettings {
    ResourceSdkSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn call(provider: &str) -> RunThreadCall {
    RunThreadCall {
        provider: String::from(provider),
        thread_id: String::from("thread_a"),
        assistant_id: String::from("asst_a"),
        additional_instructions: None,
        instructions: None,
        metadata: None,
        model: None,
        stream: None,
        tools: None,
        client: None,
        event_handler: None,
        arun_thread: None,
        params: RunParams::default(),
    }
}

// a base with every key named: the ast-grep rules refuse ..RunParams::default() in a literal
fn run_params() -> RunParams {
    RunParams {
        api_base: None,
        api_key: None,
        api_version: None,
        organization: None,
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_username: None,
        azure_password: None,
        azure_scope: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        extra_body: None,
    }
}

fn azure_call(params: RunParams) -> RunThreadCall {
    let mut input = call("azure");
    input.params = params;
    input
}

// the three stream states a case can set without naming Option<Option<RunFlag>>
#[derive(Debug)]
enum StreamArg {
    Bool(bool),
    Null,
    Number(f64),
    Text(&'static str),
}

impl StreamArg {
    fn apply(self, input: &mut RunThreadCall) {
        input.stream = match self {
            Self::Bool(value) => Some(Some(RunFlag::Bool(value))),
            Self::Null => Some(None),
            Self::Number(value) => Some(Some(RunFlag::Number(value))),
            Self::Text(value) => Some(Some(RunFlag::Text(String::from(value)))),
        };
    }
}

fn recording_handler() -> Arc<RecordingHandler> {
    Arc::new(RecordingHandler::default())
}

fn run_reply(body: &str) -> RunReply {
    serde_json::from_str(body).unwrap()
}

fn polled(outcome: RunOutcome<RunStream<Recorder>>) -> Option<RunReply> {
    match outcome {
        RunOutcome::Polled(reply) => *reply,
        RunOutcome::Streaming(_) => panic!("the call streamed instead of polling"),
    }
}

fn manager(outcome: RunOutcome<RunStream<Recorder>>) -> RunStream<Recorder> {
    match outcome {
        RunOutcome::Streaming(stream) => stream,
        RunOutcome::Polled(_) => panic!("the call polled instead of streaming"),
    }
}

fn failure(error: &AssistantRunError) -> &RunFailure {
    match error {
        AssistantRunError::Mapped(failure) => failure,
        AssistantRunError::Unsupported { .. }
        | AssistantRunError::DuplicateStream
        | AssistantRunError::Timeout(_)
        | AssistantRunError::InvalidTimeout { .. }
        | AssistantRunError::InvalidTimeoutText { .. }
        | AssistantRunError::InvalidExtraBody
        | AssistantRunError::Dispatch(_) => panic!("the call did not map its failure: {error}"),
    }
}

fn openai_recorder() -> (Arc<Recorder>, Fixture) {
    (Arc::new(Recorder::default()), Fixture::new())
}

async fn event_names(stream: RunStream<Recorder>) -> Vec<String> {
    let mut events = stream.events().await.unwrap();
    let mut names = Vec::new();
    while let Some(item) = events.next().await {
        names.push(item.unwrap().name().to_owned());
    }
    names
}

fn completed_event_frames() -> Vec<Vec<u8>> {
    let created = format!("data: {{\"event\":\"thread.run.created\",\"data\":{QUEUED}}}\n\n");
    let completed =
        format!("data: {{\"event\":\"thread.run.completed\",\"data\":{COMPLETED}}}\n\n");
    vec![
        created.into_bytes(),
        b"data: {\"event\":\"thread.message.delta\",\"data\":{\"id\":\"msg_1\"}}\n\n".to_vec(),
        completed.into_bytes(),
        b"data: [DONE]\n\n".to_vec(),
    ]
}

fn joined(frames: Vec<Vec<u8>>) -> Vec<u8> {
    frames.into_iter().flatten().collect()
}

fn polled_outcome(outcome: RunOutcome<()>) -> Option<RunReply> {
    match outcome {
        RunOutcome::Polled(reply) => *reply,
        RunOutcome::Streaming(()) => panic!("the call streamed instead of polling"),
    }
}

#[rstest]
#[case::completed(COMPLETED)]
#[case::requires_action(REQUIRES_ACTION)]
#[case::failed(FAILED)]
#[tokio::test]
async fn terminal_runs_return_unchanged(#[case] body: &str) {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(body);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let reply = polled(sdk.run_thread(call("openai")).await.unwrap()).unwrap();
    assert_eq!(reply, run_reply(body));
}

#[tokio::test]
async fn requires_action_and_failed_keep_their_details() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue_twice(REQUIRES_ACTION);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let action = polled(sdk.run_thread(call("openai")).await.unwrap()).unwrap();
    assert_eq!(action.status, "requires_action");
    let required = action.required_action.unwrap();
    assert_eq!(
        required["submit_tool_outputs"]["tool_calls"][0]["id"],
        "call_1"
    );
    let (failed_recorder, failed_fixture) = openai_recorder();
    failed_recorder.queue_twice(FAILED);
    let failed_sdk = failed_fixture.sdk(ProviderRuns {
        transport: Arc::clone(&failed_recorder),
    });
    let failed = polled(failed_sdk.run_thread(call("openai")).await.unwrap()).unwrap();
    assert_eq!(failed.last_error.unwrap()["code"], "server_error");
}

#[rstest]
#[case::default(None, None, Duration::from_secs(600))]
#[case::zero_falls_back(
    Some(RunTimeout::Plain(TimeoutValue::Number(0.0_f64))),
    Some(6.0_f64),
    Duration::from_secs(6)
)]
#[case::number(
    Some(RunTimeout::Plain(TimeoutValue::Number(2.5_f64))),
    None,
    Duration::from_millis(2500)
)]
#[case::text(
    Some(RunTimeout::Plain(TimeoutValue::String(String::from("7.5")))),
    None,
    Duration::from_millis(7500)
)]
#[case::empty_text_falls_back(
    Some(RunTimeout::Plain(TimeoutValue::String(String::new()))),
    Some(8.0_f64),
    Duration::from_secs(8)
)]
#[case::structured(
    Some(RunTimeout::Structured(StructuredTimeout {
        connect: None,
        read: Some(4.5_f64),
        write: None,
        pool: None,
    })),
    None,
    Duration::from_millis(4500)
)]
#[tokio::test]
async fn openai_timeout_selection(
    #[case] timeout: Option<RunTimeout>,
    #[case] request_timeout: Option<f64>,
    #[case] expected: Duration,
) {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut input = call("openai");
    input.params.timeout = timeout;
    input.params.request_timeout = request_timeout;
    sdk.run_thread(input).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::OpenAi { options, .. } => assert_eq!(options.timeout, Some(expected)),
        CapturedTarget::Azure { .. } => panic!("the openai provider built an azure target"),
    }
}

#[tokio::test]
async fn an_openai_client_under_azure_keeps_the_openai_protocol() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let sink = recording_handler();
    let mut input = azure_call(RunParams::default());
    input.event_handler = Some(sink);
    input.client = Some(RunClient {
        class: RunClientClass::OpenAi,
        api_key: Some(secret("client-key")),
        base_url: Some(String::from(BASE)),
        api_version: None,
        azure_ad_token: None,
        timeout: None,
        max_retries: None,
        organization: None,
    });
    sdk.run_thread(input).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::OpenAi {
            handler, supplied, ..
        } => {
            assert!(!handler);
            assert_eq!(supplied.unwrap().class, RunClientClass::OpenAi);
        }
        CapturedTarget::Azure { .. } => panic!("the supplied openai client built an azure target"),
    }
}

#[rstest]
#[case::absent(None, None)]
#[case::empty_object(Some(json("{}")), None)]
#[case::null_uses_the_env(Some(JsonValue::Null), Some("env-token"))]
#[case::body_token(Some(json(r#"{"azure_ad_token": "body-token"}"#)), Some("body-token"))]
#[case::body_null(Some(json(r#"{"azure_ad_token": null}"#)), None)]
#[tokio::test]
async fn azure_extra_body_token_choices(
    #[case] extra_body: Option<JsonValue>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = Fixture::new();
    fixture.settings.azure_ad_token = Some(secret("env-token"));
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut params = run_params();
    params.extra_body = extra_body;
    sdk.run_thread(azure_call(params)).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::Azure { azure_ad_token, .. } => {
            assert_eq!(
                azure_ad_token.as_ref().map(ExposeSecret::expose_secret),
                expected
            );
        }
        CapturedTarget::OpenAi { .. } => panic!("the azure provider built an openai target"),
    }
}

#[tokio::test]
async fn azure_credentials_reach_the_litellm_params() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut credentials = run_params();
    credentials.tenant_id = Some(String::from("tenant"));
    credentials.client_id = Some(String::from("client"));
    credentials.client_secret = Some(secret("client-secret"));
    credentials.azure_username = Some(String::from("user"));
    credentials.azure_password = Some(secret("password"));
    credentials.azure_scope = Some(String::from("scope"));
    sdk.run_thread(azure_call(credentials)).await.unwrap();
    match sdk.operations.targets().remove(0) {
        CapturedTarget::Azure { params, .. } => {
            assert_eq!(params.auth.tenant_id.as_deref(), Some("tenant"));
            assert_eq!(params.auth.client_id.as_deref(), Some("client"));
            assert_eq!(
                params
                    .auth
                    .client_secret
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                Some("client-secret")
            );
            assert_eq!(params.auth.azure_username.as_deref(), Some("user"));
            assert_eq!(
                params
                    .auth
                    .azure_password
                    .as_ref()
                    .map(ExposeSecret::expose_secret),
                Some("password")
            );
            assert_eq!(params.auth.azure_scope.as_deref(), Some("scope"));
        }
        CapturedTarget::OpenAi { .. } => panic!("the azure provider built an openai target"),
    }
}

#[tokio::test]
async fn unsupported_provider_reports_the_litellm_bad_request() {
    let (recorder, fixture) = openai_recorder();
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let error = sdk.run_thread(call("anthropic")).await.err().unwrap();
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'run_thread'. Only 'openai' is supported."
    );
    assert_eq!(
        error.response(),
        Some(("Unsupported provider", "CREATE_THREAD"))
    );
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn sync_status_failure_keeps_the_sdk_body() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(404, NOT_FOUND);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let error = sdk.run_thread(call("openai")).await.err().unwrap();
    let AssistantRunError::Dispatch(failure) = error else {
        panic!("the sync call mapped its failure");
    };
    let (status, body) = failure.status_and_body().unwrap();
    assert_eq!(status, 404);
    assert_eq!(body, NOT_FOUND.as_bytes());
    assert!(failure.message().starts_with("Error code: 404"));
}

#[tokio::test]
async fn arun_status_failure_maps_through_b011() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(404, NOT_FOUND);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let error = sdk.arun_thread(call("openai")).await.err().unwrap();
    let mapped = failure(&error);
    assert_eq!(mapped.class_name, "NotFoundError");
    assert!(matches!(mapped.class, ErrorClass::NotFound(_)));
    let metadata = mapped.class.metadata();
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert!(metadata.openai_code.is_none());
    assert_eq!(
        mapped.message,
        "litellm.NotFoundError: NotFoundError: OpenAIException - no such run"
    );
}

#[tokio::test]
async fn arun_thread_maps_a_wrapper_failure_as_api_connection() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut input = call("openai");
    input.params.timeout = Some(RunTimeout::Plain(TimeoutValue::String(String::from("x"))));
    let error = sdk.arun_thread(input).await.err().unwrap();
    let mapped = failure(&error);
    assert_eq!(mapped.class_name, "APIConnectionError");
    assert!(matches!(mapped.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn arun_thread_maps_a_negative_timeout_to_the_timeout_class() {
    let fixture = Fixture::new();
    let operations = CaptureRuns::default();
    let sdk = fixture.sdk(operations);
    let mut input = call("openai");
    input.params.timeout = Some(RunTimeout::Plain(TimeoutValue::Number(-1.0_f64)));
    let error = sdk.arun_thread(input).await.err().unwrap();
    let mapped = failure(&error);
    assert_eq!(mapped.class_name, "Timeout");
    assert!(matches!(mapped.class, ErrorClass::Timeout(_)));
    assert_eq!(mapped.class.metadata().status_code, 408);
    assert_eq!(
        mapped.message,
        "litellm.Timeout: APITimeoutError - Request timed out. Error_str: Request timed out."
    );
}

#[tokio::test]
async fn a_server_error_body_reaches_the_mapped_error() {
    let (recorder, fixture) = openai_recorder();
    recorder.queue(500, SERVER_ERROR);
    let sdk = fixture.sdk(ProviderRuns {
        transport: Arc::clone(&recorder),
    });
    let error = sdk.arun_thread(call("openai")).await.err().unwrap();
    let mapped = failure(&error);
    let metadata = mapped.class.metadata();
    assert_eq!(
        metadata.error_type,
        WireField::Text(String::from("internal_server_error"))
    );
    assert_eq!(metadata.param, WireField::Text(String::from("model")));
    assert_eq!(metadata.openai_code.as_deref(), Some("server_boom"));
}

mod polls;
mod streams;
