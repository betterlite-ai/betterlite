use std::collections::VecDeque;
use std::future::pending;
use std::io;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::bedrock_settings::{AwsSettings, SessionTagValue};
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, RealtimeSuccessQueue,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use uuid::Uuid;

use super::logging::{FakeLogging, FakeQueue};
use crate::agentcore_stream::IdSource;
use crate::aws_settings::AuthParams;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use crate::realtime_events::scope::EventScope;
use crate::realtime_events::stream::{
    BedrockBidirectionalStream, BedrockInputStream, BedrockOutputChunk, BedrockOutputStream,
    BedrockPayloadPart, ClosableBedrockRuntimeClient, StreamError,
};
use crate::realtime_transport::error::{ProviderClass, RealtimeTransportError};
use crate::realtime_transport::prepare::RealtimeCredentials;
use crate::realtime_transport::runtime::{RealtimeClient, RealtimeRuntime, RuntimeConfig};
use crate::realtime_transport::session::{RealtimeCall, RealtimeTransport};
use crate::signing::target::RegionInputs;

pub(super) const MODEL: &str = "amazon.nova-sonic-v1:0";
pub(super) const TRACE_ID: &str = "trace-realtime-0001";
pub(super) const ACCESS: &str = "AKIDBETTERLITEFIXTURE";
pub(super) const SECRET: &str = "fixture/secret/key/+EXAMPLEKEY0000";
pub(super) const TOKEN: &str = "fixture-session-token-0001";
pub(super) const RESOLVED_ACCESS: &str = "AKIDRESOLVEDFIXTURE";
pub(super) const RESOLVED_SECRET: &str = "resolved/secret/key/+EXAMPLEKEY00";
pub(super) const RESOLVED_TOKEN: &str = "resolved-session-token-0001";
pub(super) const REGION: &str = "us-east-1";

pub(super) const SESSION_UPDATE: &str = r#"{"type":"session.update","session":{}}"#;
pub(super) const CONTENT_START: &str =
    r#"{"event":{"contentStart":{"role":"ASSISTANT","type":"TEXT"}}}"#;
pub(super) const TEXT_OUTPUT: &str = r#"{"event":{"textOutput":{"content":"Hello"}}}"#;
pub(super) const CONTENT_END: &str = r#"{"event":{"contentEnd":{"stopReason":"END_TURN"}}}"#;
pub(super) const USAGE_EVENT: &str = r#"{"event":{"usageEvent":{"totalInputTokens":10,"totalOutputTokens":4,"totalTokens":14,"details":{"total":{"input":{"speechTokens":2,"textTokens":8},"output":{"speechTokens":1,"textTokens":3}}}}}}"#;

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub(super) struct FakeError {
    class_name: Option<String>,
    message: String,
}

impl FakeError {
    pub(super) fn new(class_name: &str, message: &str) -> Self {
        Self {
            class_name: Some(class_name.to_owned()),
            message: message.to_owned(),
        }
    }
}

impl ProviderClass for FakeError {
    fn provider_class_name(&self) -> Option<&str> {
        self.class_name.as_deref()
    }

    fn provider_message(&self) -> Option<&str> {
        Some(&self.message)
    }
}

pub(super) fn stream_error(message: &str, source: FakeError) -> StreamError<FakeError> {
    StreamError {
        message: message.to_owned(),
        source: Some(source),
    }
}

fn close_error() -> StreamError<FakeError> {
    stream_error(
        "the SDK close failed",
        FakeError::new("CloseError", "the SDK close failed"),
    )
}

pub(super) struct CountingIds {
    next: Mutex<u128>,
}

impl CountingIds {
    pub(super) const fn new() -> Self {
        Self {
            next: Mutex::new(1),
        }
    }
}

impl IdSource for CountingIds {
    fn next_id(&self) -> Uuid {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let id = Uuid::from_u128(*next);
        *next = next.saturating_add(1);
        id
    }
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor")
}

pub(super) fn credentials(access: &str, secret: &str, token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access.to_owned()),
        secret_access_key: SecretString::from(secret.to_owned()),
        session_token: token.map(|value| SecretString::from(value.to_owned())),
    }
}

pub(super) fn auth(access: &str, secret: &str, token: Option<&str>) -> AuthParams {
    AuthParams {
        aws_access_key_id: Some(SecretString::from(access.to_owned())),
        aws_secret_access_key: Some(SecretString::from(secret.to_owned())),
        aws_session_token: token.map(|value| SecretString::from(value.to_owned())),
        aws_session_name: None,
        aws_profile_name: None,
        aws_role_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        aws_external_id: None,
        aws_session_tags: None,
    }
}

pub(super) fn full_auth() -> AuthParams {
    AuthParams {
        aws_session_name: Some("fixture-session".to_owned()),
        aws_profile_name: Some("fixture-profile".to_owned()),
        aws_role_name: Some("fixture-role".to_owned()),
        aws_web_identity_token: Some(SecretString::from("fixture-web-identity".to_owned())),
        aws_sts_endpoint: Some("https://sts.fixture.example".to_owned()),
        aws_external_id: Some("fixture-external".to_owned()),
        aws_session_tags: Some(full_session_tags()),
        ..auth(ACCESS, SECRET, Some(TOKEN))
    }
}

pub(super) fn full_session_tags() -> SessionTagValue {
    SessionTagValue::List(vec![SessionTagValue::Mapping(IndexMap::from([
        ("Key".to_owned(), SessionTagValue::Text("team".to_owned())),
        ("Value".to_owned(), SessionTagValue::Text("core".to_owned())),
    ]))])
}

pub(super) fn secret_text(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedConfig {
    pub(super) endpoint_uri: String,
    pub(super) region: String,
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    pub(super) session_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RecordedResolve {
    pub(super) region: String,
    pub(super) access_key_id: Option<String>,
    pub(super) secret_access_key: Option<String>,
    pub(super) session_token: Option<String>,
    pub(super) region_name: Option<String>,
    pub(super) session_name: Option<String>,
    pub(super) profile_name: Option<String>,
    pub(super) role_name: Option<String>,
    pub(super) web_identity_token: Option<String>,
    pub(super) sts_endpoint: Option<String>,
    pub(super) external_id: Option<String>,
    pub(super) session_tags: Option<SessionTagValue>,
    pub(super) role_arn: Option<String>,
    pub(super) web_identity_token_file: Option<String>,
    pub(super) bearer_token: Option<String>,
}

impl RecordedResolve {
    fn new(settings: &AwsSettings, region: &str) -> Self {
        Self {
            region: region.to_owned(),
            access_key_id: secret_text(settings.access_key_id.as_ref()),
            secret_access_key: secret_text(settings.secret_access_key.as_ref()),
            session_token: secret_text(settings.session_token.as_ref()),
            region_name: settings.region_name.clone(),
            session_name: settings.session_name.clone(),
            profile_name: settings.profile_name.clone(),
            role_name: settings.role_name.clone(),
            web_identity_token: secret_text(settings.web_identity_token.as_ref()),
            sts_endpoint: settings.sts_endpoint.clone(),
            external_id: settings.external_id.clone(),
            session_tags: settings.session_tags.clone(),
            role_arn: settings.role_arn.clone(),
            web_identity_token_file: settings.web_identity_token_file.clone(),
            bearer_token: secret_text(settings.bearer_token.as_ref()),
        }
    }
}

enum ResolveMode {
    Fixed(Result<Option<Credentials>, CredentialError>),
    Echo,
}

impl Default for ResolveMode {
    fn default() -> Self {
        Self::Fixed(Ok(None))
    }
}

#[derive(Default)]
struct ResolveState {
    mode: Mutex<ResolveMode>,
    calls: Mutex<Vec<RecordedResolve>>,
}

pub(super) struct FakeCredentials {
    state: Arc<ResolveState>,
}

impl FakeCredentials {
    pub(super) fn returning(credentials: Credentials) -> Self {
        Self::fixed(Ok(Some(credentials)))
    }

    pub(super) fn absent() -> Self {
        Self::fixed(Ok(None))
    }

    pub(super) fn failing(error: CredentialError) -> Self {
        Self::fixed(Err(error))
    }

    pub(super) fn echoing() -> Self {
        Self::with_mode(ResolveMode::Echo)
    }

    fn fixed(outcome: Result<Option<Credentials>, CredentialError>) -> Self {
        Self::with_mode(ResolveMode::Fixed(outcome))
    }

    fn with_mode(mode: ResolveMode) -> Self {
        let state = ResolveState {
            mode: Mutex::new(mode),
            calls: Mutex::new(Vec::new()),
        };
        Self {
            state: Arc::new(state),
        }
    }

    pub(super) fn calls(&self) -> Vec<RecordedResolve> {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RealtimeCredentials for FakeCredentials {
    async fn resolve(
        &self,
        settings: &AwsSettings,
        region: &str,
    ) -> Result<Option<Credentials>, CredentialError> {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedResolve::new(settings, region));
        let mut mode = self
            .state
            .mode
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match &mut *mode {
            ResolveMode::Fixed(outcome) => {
                mem::replace(outcome, Err(CredentialError::MissingCredentials))
            }
            ResolveMode::Echo => match (&settings.access_key_id, &settings.secret_access_key) {
                (Some(access), Some(secret)) => Ok(Some(Credentials {
                    access_key_id: access.clone(),
                    secret_access_key: secret.clone(),
                    session_token: settings.session_token.clone(),
                })),
                _ => Ok(None),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Chunk {
    Text(&'static str),
    End,
}

#[derive(Clone, Debug, Default)]
pub(super) struct StreamPlan {
    chunks: Vec<Chunk>,
    receive_failure: Option<usize>,
    block_receive: bool,
    send_failures: Vec<usize>,
}

impl StreamPlan {
    pub(super) fn new(chunks: Vec<Chunk>) -> Self {
        Self {
            chunks,
            receive_failure: None,
            block_receive: false,
            send_failures: Vec::new(),
        }
    }

    pub(super) fn failing_receive(mut self, index: usize) -> Self {
        self.receive_failure = Some(index);
        self
    }

    pub(super) fn failing_send(mut self, index: usize) -> Self {
        self.send_failures.push(index);
        self
    }

    pub(super) fn blocking(mut self) -> Self {
        self.block_receive = true;
        self
    }
}

#[derive(Clone)]
pub(super) struct StreamProfile {
    sent: Arc<Mutex<Vec<Json>>>,
    closed: Arc<AtomicBool>,
}

impl StreamProfile {
    pub(super) fn sent(&self) -> Vec<Json> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct RuntimeState {
    configs: Mutex<Vec<RecordedConfig>>,
    invocations: Mutex<Vec<String>>,
    plan: Mutex<StreamPlan>,
    connect_failure: Mutex<Option<StreamError<FakeError>>>,
    invoke_failure: Mutex<Option<StreamError<FakeError>>>,
    stream: Mutex<Option<StreamProfile>>,
    closes: AtomicUsize,
    close_failure: AtomicBool,
}

#[derive(Default)]
pub(super) struct FakeRuntime {
    state: Arc<RuntimeState>,
}

impl FakeRuntime {
    pub(super) fn with_plan(mut self, plan: StreamPlan) -> Self {
        self.state_mut().plan = Mutex::new(plan);
        self
    }

    pub(super) fn failing_connect(mut self, error: StreamError<FakeError>) -> Self {
        self.state_mut().connect_failure = Mutex::new(Some(error));
        self
    }

    pub(super) fn failing_invoke(mut self, error: StreamError<FakeError>) -> Self {
        self.state_mut().invoke_failure = Mutex::new(Some(error));
        self
    }

    pub(super) fn failing_close(mut self) -> Self {
        self.state_mut().close_failure = AtomicBool::new(true);
        self
    }

    fn state_mut(&mut self) -> &mut RuntimeState {
        Arc::get_mut(&mut self.state).expect("the only runtime state reference")
    }

    pub(super) fn configs(&self) -> Vec<RecordedConfig> {
        self.state
            .configs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn invocations(&self) -> Vec<String> {
        self.state
            .invocations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn closes(&self) -> usize {
        self.state.closes.load(Ordering::SeqCst)
    }

    pub(super) fn stream(&self) -> StreamProfile {
        let stream = self
            .state
            .stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        stream.expect("the runtime opened a stream")
    }
}

impl RealtimeRuntime for FakeRuntime {
    type Error = FakeError;

    async fn connect(
        &self,
        config: RuntimeConfig,
    ) -> Result<Box<dyn RealtimeClient<Error = FakeError>>, StreamError<FakeError>> {
        let recorded = RecordedConfig {
            endpoint_uri: config.endpoint_uri.clone(),
            region: config.region.clone(),
            access_key_id: config.credentials.access_key_id.expose_secret().to_owned(),
            secret_access_key: config
                .credentials
                .secret_access_key
                .expose_secret()
                .to_owned(),
            session_token: secret_text(config.credentials.session_token.as_ref()),
        };
        self.state
            .configs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        let failure = self
            .state
            .connect_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(Box::new(FakeRuntimeClient {
            state: Arc::clone(&self.state),
        }))
    }
}

struct FakeRuntimeClient {
    state: Arc<RuntimeState>,
}

#[async_trait]
impl ClosableBedrockRuntimeClient for FakeRuntimeClient {
    type Error = FakeError;

    async fn close(&self) -> Result<(), StreamError<FakeError>> {
        let _previous = self.state.closes.fetch_add(1, Ordering::SeqCst);
        if self.state.close_failure.load(Ordering::SeqCst) {
            return Err(close_error());
        }
        Ok(())
    }
}

#[async_trait]
impl RealtimeClient for FakeRuntimeClient {
    type Error = FakeError;

    fn closable(&self) -> &dyn ClosableBedrockRuntimeClient<Error = FakeError> {
        self
    }

    async fn invoke(
        &self,
        model_id: &str,
    ) -> Result<Box<dyn BedrockBidirectionalStream<Error = FakeError>>, StreamError<FakeError>>
    {
        self.state
            .invocations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_id.to_owned());
        let failure = self
            .state
            .invoke_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        let plan = self
            .state
            .plan
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let stream = ScriptedStream::new(plan);
        let profile = stream.profile();
        *self
            .state
            .stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(profile);
        Ok(Box::new(stream))
    }
}

struct ScriptedInput {
    sent: Arc<Mutex<Vec<Json>>>,
    send_failures: Vec<usize>,
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl BedrockInputStream for ScriptedInput {
    type Error = FakeError;

    async fn send(&self, event: Json) -> Result<(), StreamError<FakeError>> {
        let index = {
            let mut sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
            let position = sent.len();
            sent.push(event);
            position
        };
        if self.send_failures.contains(&index) {
            return Err(stream_error(
                "the input send failed",
                FakeError::new("ClientStreamError", "the input send failed"),
            ));
        }
        Ok(())
    }

    async fn close(&self) -> Result<(), StreamError<FakeError>> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct ScriptedPart(Option<Vec<u8>>);

impl BedrockPayloadPart for ScriptedPart {
    fn bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }
}

struct ScriptedChunk {
    part: ScriptedPart,
}

impl BedrockOutputChunk for ScriptedChunk {
    fn value(&self) -> Option<&dyn BedrockPayloadPart> {
        let part: &dyn BedrockPayloadPart = &self.part;
        Some(part)
    }
}

struct OutputState {
    chunks: Mutex<VecDeque<Chunk>>,
    receives: Arc<AtomicUsize>,
    receive_failure: Option<usize>,
    block_receive: bool,
}

pub(super) struct ScriptedStream {
    input: ScriptedInput,
    output: Arc<OutputState>,
}

impl ScriptedStream {
    fn new(plan: StreamPlan) -> Self {
        Self {
            input: ScriptedInput {
                sent: Arc::new(Mutex::new(Vec::new())),
                send_failures: plan.send_failures,
                closed: Arc::new(AtomicBool::new(false)),
            },
            output: Arc::new(OutputState {
                chunks: Mutex::new(plan.chunks.into_iter().collect()),
                receives: Arc::new(AtomicUsize::new(0)),
                receive_failure: plan.receive_failure,
                block_receive: plan.block_receive,
            }),
        }
    }

    fn profile(&self) -> StreamProfile {
        StreamProfile {
            sent: Arc::clone(&self.input.sent),
            closed: Arc::clone(&self.input.closed),
        }
    }
}

#[async_trait]
impl BedrockBidirectionalStream for ScriptedStream {
    type Error = FakeError;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = FakeError> {
        &self.input
    }

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = FakeError>>), StreamError<FakeError>>
    {
        let output: Box<dyn BedrockOutputStream<Error = FakeError>> = Box::new(ScriptedOutput {
            output: Arc::clone(&self.output),
        });
        Ok((Json::Null, output))
    }
}

struct ScriptedOutput {
    output: Arc<OutputState>,
}

#[async_trait]
impl BedrockOutputStream for ScriptedOutput {
    type Error = FakeError;

    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<FakeError>> {
        let index = self.output.receives.fetch_add(1, Ordering::SeqCst);
        if self.output.receive_failure == Some(index) {
            return Err(stream_error(
                "the provider receive failed",
                FakeError::new("ModelStreamErrorException", "the provider stream broke"),
            ));
        }
        let chunk = self
            .output
            .chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match chunk {
            Some(Chunk::Text(text)) => Ok(Some(Box::new(ScriptedChunk {
                part: ScriptedPart(Some(text.as_bytes().to_vec())),
            }))),
            Some(Chunk::End) => Ok(None),
            None => {
                if self.output.block_receive {
                    pending().await
                } else {
                    Ok(None)
                }
            }
        }
    }
}

pub(super) enum ReceiveStep {
    Frame(&'static str),
    Pending,
}

#[derive(Default)]
struct ClientState {
    receives: Mutex<VecDeque<ReceiveStep>>,
    sent: Mutex<Vec<String>>,
    send_failure: Option<usize>,
    closes: Mutex<Vec<(u16, Option<String>)>>,
}

#[derive(Default)]
pub(super) struct ScriptedClient {
    state: Arc<ClientState>,
}

impl ScriptedClient {
    pub(super) fn scripted(steps: Vec<ReceiveStep>) -> Self {
        let client = Self::default();
        *client
            .state
            .receives
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = steps.into_iter().collect();
        client
    }

    pub(super) fn failing_send(mut self, index: usize) -> Self {
        Arc::get_mut(&mut self.state)
            .expect("the only client state reference")
            .send_failure = Some(index);
        self
    }

    pub(super) fn sends(&self) -> Vec<String> {
        self.state
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn sent_events(&self) -> Vec<Json> {
        self.sends()
            .iter()
            .map(|text| serde_json::from_str(text).expect("a JSON client event"))
            .collect()
    }

    pub(super) fn closes(&self) -> Vec<(u16, Option<String>)> {
        self.state
            .closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RealtimeClientWebSocket for ScriptedClient {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        let step = self
            .state
            .receives
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match step {
            Some(ReceiveStep::Frame(text)) => Ok(text.to_owned()),
            Some(ReceiveStep::Pending) => pending().await,
            None => Err(ClientError {
                message: "the client is gone".to_owned(),
                source: None,
            }),
        }
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        let index = {
            let mut sent = self
                .state
                .sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let position = sent.len();
            sent.push(data);
            position
        };
        if self.state.send_failure == Some(index) {
            return Err(ClientError {
                message: "the client send failed".to_owned(),
                source: None,
            });
        }
        Ok(())
    }

    async fn close(&self, code: u16, reason: Option<String>) {
        self.state
            .closes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((code, reason));
    }
}

pub(super) struct Options<'call> {
    pub(super) model: &'call str,
    pub(super) pending: Option<&'call str>,
    pub(super) committed: bool,
    pub(super) committed_failure: Option<&'call str>,
    pub(super) api_base: Option<&'call str>,
    pub(super) endpoint: Option<&'call str>,
    pub(super) region: Option<&'call str>,
    pub(super) region_inputs: RegionInputs<'call>,
    pub(super) auth: AuthParams,
    pub(super) logged_types: LoggedEventTypes,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            model: MODEL,
            pending: None,
            committed: false,
            committed_failure: None,
            api_base: None,
            endpoint: None,
            region: Some(REGION),
            region_inputs: RegionInputs::default(),
            auth: AuthParams::default(),
            logged_types: LoggedEventTypes::Default,
        }
    }
}

pub(super) struct RunResult {
    pub(super) outcome: Result<(), RealtimeTransportError<FakeError, io::Error>>,
    pub(super) scope: EventScope,
}

fn resolved_credentials() -> FakeCredentials {
    FakeCredentials::returning(credentials(
        RESOLVED_ACCESS,
        RESOLVED_SECRET,
        Some(RESOLVED_TOKEN),
    ))
}

pub(super) struct Fixture {
    pub(super) ids: CountingIds,
    pub(super) redactor: Redactor,
    pub(super) client: ScriptedClient,
    pub(super) runtime: FakeRuntime,
    pub(super) credentials: FakeCredentials,
    pub(super) logging: FakeLogging,
    pub(super) queue: FakeQueue,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let ids = CountingIds::new();
        let redactor = redactor();
        let client = ScriptedClient::scripted(Vec::new());
        let runtime = FakeRuntime::default();
        let credentials = resolved_credentials();
        Self {
            ids,
            redactor,
            client,
            runtime,
            credentials,
            logging: FakeLogging::default(),
            queue: FakeQueue::new(true),
        }
    }

    pub(super) async fn run(&mut self, options: Options<'_>) -> RunResult {
        let mut scope = EventScope {
            pending_session_update: options
                .pending
                .map_or(Json::Null, |text| Json::Text(text.to_owned())),
            session_committed: Json::Bool(options.committed),
            committed_failure: options
                .committed_failure
                .map_or(Json::Null, |text| Json::Text(text.to_owned())),
        };
        let client: &dyn RealtimeClientWebSocket<Error = io::Error> = &self.client;
        let logging: &dyn RealtimeLogging = &self.logging;
        let queue: &dyn RealtimeSuccessQueue = &self.queue;
        let transport = RealtimeTransport::new(&self.runtime, &self.credentials);
        let outcome = transport
            .realtime(RealtimeCall {
                model: options.model,
                client,
                logging,
                logging_worker: Some(queue),
                api_base: options.api_base,
                aws_bedrock_runtime_endpoint: options.endpoint,
                aws_region_name: options.region,
                auth: options.auth,
                region_inputs: options.region_inputs,
                scope: &mut scope,
                ids: &self.ids,
                redactor: &self.redactor,
                logged_types: &options.logged_types,
                trace_id: TRACE_ID,
            })
            .await;
        RunResult { outcome, scope }
    }

    pub(super) fn last_config(&self) -> RecordedConfig {
        self.runtime
            .configs()
            .pop()
            .expect("the runtime received a configuration")
    }
}

pub(super) fn event_type(event: &Json) -> Option<&str> {
    let Json::Object(fields) = event else {
        return None;
    };
    match fields.get("type") {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}

pub(super) fn field<'value>(event: &'value Json, name: &str) -> Option<&'value Json> {
    let Json::Object(fields) = event else {
        return None;
    };
    fields.get(name)
}

// The Bedrock input frames carry their event name as the only key under "event"
pub(super) fn event_key(value: &Json) -> Option<&str> {
    let Json::Object(event) = field(value, "event")? else {
        return None;
    };
    event.keys().next().map(String::as_str)
}
