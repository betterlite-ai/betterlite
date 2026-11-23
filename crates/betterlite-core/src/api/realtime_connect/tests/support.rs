use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::api::realtime_connect::RealtimeConnectionSettings;
use betterlite_context::resource_config_selection::realtime::RealtimeConfigTag;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::realtime_backend_transport::dial::{
    ConnectedBackend, DialRequest, RealtimeWebSocketDialer,
};
use crate::api::realtime_backend_transport::error::DialError;
use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use crate::dispatch::error::DispatchError;

use super::super::dispatch::arealtime;
use super::super::token::{VertexAccessTokenSource, VertexTokenError, VertexTokenFuture};
use super::super::{
    AzureAdTokenRequest, RealtimeConnectCall, RealtimeConnectDispatch, RealtimeConnectError,
    RealtimeConnectParams, RealtimeConnection,
};

type TestError = IoError;

pub(super) struct TestBackend;

#[async_trait]
impl RealtimeBackend for TestBackend {
    async fn send(&mut self, _message: RealtimeMessage) -> Result<(), WebSocketError> {
        Ok(())
    }

    async fn recv(&mut self, _decode: Option<bool>) -> Result<RealtimeMessage, WebSocketError> {
        Ok(RealtimeMessage::Text(String::new()))
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        Ok(())
    }
}

pub(super) struct TestConfig;

#[async_trait]
impl BaseRealtimeConfig for TestConfig {
    type Error = TestError;
    type Backend = TestBackend;

    fn validate_environment(
        &self,
        environment: &Environment<'_>,
    ) -> Result<HeaderMap, Self::Error> {
        Ok(environment.headers.clone())
    }

    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Self::Error> {
        Ok(Vec::new())
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Self::Error> {
        Ok(RealtimeResponseTypedDict {
            response: Json::Null,
            current_output_item_id: None,
            current_response_id: None,
            current_delta_chunks: None,
            current_conversation_id: None,
            current_item_chunks: None,
            current_delta_type: None,
            session_configuration_request: None,
        })
    }
}

type TokenAnswer = Result<(SecretString, String), VertexTokenError>;

#[derive(Default)]
pub(super) struct ScriptedTokens {
    answers: Mutex<VecDeque<TokenAnswer>>,
    pub(super) calls: Mutex<Vec<(Option<Json>, Option<String>)>>,
}

impl ScriptedTokens {
    pub(super) fn with(answers: Vec<TokenAnswer>) -> Arc<Self> {
        let tokens = Self::default();
        *tokens
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = VecDeque::from(answers);
        Arc::new(tokens)
    }
}

impl VertexAccessTokenSource for ScriptedTokens {
    type Error = TestError;

    fn resolve<'source>(
        &'source self,
        credentials: Option<&'source Json>,
        project_id: Option<&'source str>,
    ) -> VertexTokenFuture<'source> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((credentials.cloned(), project_id.map(str::to_owned)));
            self.answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| {
                    Err(VertexTokenError {
                        message: String::from("no answer"),
                    })
                })
        })
    }
}

#[derive(Clone, Default)]
enum AdReply {
    #[default]
    Unset,
    Answer(Option<SecretString>),
}

#[derive(Default)]
pub(super) struct Recorder {
    tags: Mutex<Vec<RealtimeConfigTag>>,
    pub(super) ad_requests: Mutex<Vec<(Option<String>, Option<String>)>>,
    ad_reply: Mutex<AdReply>,
    tokens: Mutex<Option<Arc<dyn VertexAccessTokenSource<Error = TestError>>>>,
}

impl Recorder {
    pub(super) fn with_tokens(tokens: Arc<ScriptedTokens>) -> Self {
        let recorder = Self::default();
        *recorder
            .tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(tokens);
        recorder
    }

    pub(super) fn with_ad_token(reply: Option<SecretString>) -> Self {
        let recorder = Self::default();
        *recorder
            .ad_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = AdReply::Answer(reply);
        recorder
    }

    pub(super) fn tags(&self) -> Vec<RealtimeConfigTag> {
        self.tags
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RealtimeConnectDispatch for Recorder {
    type Error = TestError;
    type ProviderConfig = TestConfig;

    fn provider_config(
        &self,
        tag: RealtimeConfigTag,
    ) -> Result<Arc<Self::ProviderConfig>, RealtimeConnectError<Self::Error>> {
        self.tags
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tag);
        Ok(Arc::new(TestConfig))
    }

    fn vertex_access_tokens(
        &self,
    ) -> Result<
        Arc<dyn VertexAccessTokenSource<Error = Self::Error>>,
        RealtimeConnectError<Self::Error>,
    > {
        self.tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(RealtimeConnectError::NotBuilt(DispatchError::NotBuilt {
                entry: "vertex_access_token_resolver",
            }))
    }

    fn azure_ad_token(
        &self,
        request: AzureAdTokenRequest<'_>,
    ) -> Result<Option<SecretString>, RealtimeConnectError<Self::Error>> {
        self.ad_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                request
                    .azure_ad_token
                    .map(|token| token.expose_secret().to_owned()),
                request.params.tenant_id.clone(),
            ));
        let reply = self
            .ad_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match reply {
            AdReply::Unset => Err(RealtimeConnectError::NotBuilt(DispatchError::NotBuilt {
                entry: "azure_ad_token_credentials",
            })),
            AdReply::Answer(answer) => Ok(answer),
        }
    }
}

pub(super) struct BareDispatch;

impl RealtimeConnectDispatch for BareDispatch {
    type Error = TestError;
    type ProviderConfig = TestConfig;
}

// The dispatch only stores the caller's client, so a dial through this one would be a defect.
pub(super) struct UnusedDialer;

#[async_trait]
impl RealtimeWebSocketDialer for UnusedDialer {
    async fn connect(&self, _request: &DialRequest<'_>) -> Result<ConnectedBackend, DialError> {
        Err(DialError::Other {
            message: String::from("the connection dispatch never dials"),
        })
    }
}

fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"realtime","call_type":"realtime","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap()
}

pub(super) async fn run<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
) -> (
    LoggingState,
    Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>,
)
where
    Dispatch: RealtimeConnectDispatch,
{
    run_with_environment(dispatch, settings, call, &BTreeMap::new()).await
}

pub(super) async fn run_with_environment<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    environment: &BTreeMap<String, String>,
) -> (
    LoggingState,
    Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>,
)
where
    Dispatch: RealtimeConnectDispatch,
{
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let mut logging = logging_state();
    let result = arealtime(dispatch, settings, &mut logging, &resolution, call).await;
    (logging, result)
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(String::from(value))
}

pub(super) fn params() -> RealtimeConnectParams {
    RealtimeConnectParams::default()
}

pub(super) fn settings() -> RealtimeConnectionSettings {
    RealtimeConnectionSettings::default()
}

pub(super) fn call<'call>(
    model: &'call str,
    connect: &'call RealtimeConnectParams,
    query: Option<&'call IndexMap<String, String>>,
    litellm_params: &'call IndexMap<String, Json>,
    client_sent_beta: bool,
) -> RealtimeConnectCall<'call> {
    RealtimeConnectCall {
        model,
        params: connect,
        query_params: query,
        client: None,
        client_sent_beta,
        litellm_params,
    }
}

pub(super) fn call_with_client<'call>(
    model: &'call str,
    connect: &'call RealtimeConnectParams,
    client: &Arc<dyn RealtimeWebSocketDialer>,
    litellm_params: &'call IndexMap<String, Json>,
) -> RealtimeConnectCall<'call> {
    let mut request = call(model, connect, None, litellm_params, false);
    request.client = Some(Arc::clone(client));
    request
}
