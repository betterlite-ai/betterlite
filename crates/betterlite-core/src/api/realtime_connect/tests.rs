use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
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
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use crate::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use crate::dispatch::error::DispatchError;

use super::token::{
    VertexAccessTokenSource, VertexTokenError, VertexTokenFuture,
    resolve_vertex_access_token_bounded,
};
use super::{
    AzureAdTokenRequest, RealtimeConnectCall, RealtimeConnectDispatch, RealtimeConnectError,
    RealtimeConnectParams, RealtimeConnection, arealtime,
};

#[derive(Debug)]
struct TestError {
    message: String,
}

impl fmt::Display for TestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TestError {}

struct TestBackend;

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

struct TestConfig;

#[async_trait]
impl BaseRealtimeConfig for TestConfig {
    type Error = TestError;
    type Backend = TestBackend;

    fn validate_environment(&self, environment: &Environment<'_>) -> Result<HeaderMap, Self::Error> {
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
        panic!("the realtime dispatch tests never transform a provider response")
    }
}

type TokenAnswer = Result<(SecretString, String), VertexTokenError>;

#[derive(Default)]
struct ScriptedTokens {
    answers: Mutex<VecDeque<TokenAnswer>>,
    calls: Mutex<Vec<(Option<Json>, Option<String>)>>,
}

impl ScriptedTokens {
    fn with(answers: Vec<TokenAnswer>) -> Arc<Self> {
        let tokens = Self::default();
        *tokens.answers.lock().unwrap_or_else(PoisonError::into_inner) =
            VecDeque::from(answers);
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
                .unwrap_or_else(|| Err(VertexTokenError { message: String::from("no answer") }))
        })
    }
}

#[derive(Default)]
struct Recorder {
    tags: Mutex<Vec<RealtimeConfigTag>>,
    ad_requests: Mutex<Vec<(Option<String>, Option<String>)>>,
    ad_reply: Mutex<Option<Option<SecretString>>>,
    tokens: Mutex<Option<Arc<ScriptedTokens>>>,
}

impl Recorder {
    fn with_tokens(tokens: Arc<ScriptedTokens>) -> Self {
        let recorder = Self::default();
        *recorder.tokens.lock().unwrap_or_else(PoisonError::into_inner) = Some(tokens);
        recorder
    }

    fn with_ad_token(reply: Option<SecretString>) -> Self {
        let recorder = Self::default();
        *recorder.ad_reply.lock().unwrap_or_else(PoisonError::into_inner) = Some(reply);
        recorder
    }

    fn tags(&self) -> Vec<RealtimeConfigTag> {
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
        match self
            .tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            Some(tokens) => Ok(tokens),
            None => Err(DispatchError::NotBuilt {
                entry: "vertex_access_token_resolver",
            }
            .into()),
        }
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
        match self
            .ad_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            Some(reply) => Ok(reply),
            None => Err(DispatchError::NotBuilt {
                entry: "azure_ad_token_credentials",
            }
            .into()),
        }
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

fn empty_litellm_params() -> IndexMap<String, Json> {
    IndexMap::new()
}

async fn run(
    dispatch: &Recorder,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
) -> (LoggingState, Result<RealtimeConnection<TestConfig>, RealtimeConnectError<TestError>>) {
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let mut logging = logging_state();
    let result = arealtime(dispatch, settings, &mut logging, &resolution, call).await;
    (logging, result)
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

fn text(value: &str) -> Json {
    Json::Text(String::from(value))
}

fn params() -> RealtimeConnectParams {
    RealtimeConnectParams::default()
}

#[tokio::test]
async fn realtime_generic_dispatch() {
    let dispatch = Recorder::default();
    let mut connect = params();
    connect.api_base = Some(String::from("https://named.example/v1"));
    connect.api_key = Some(secret("sk-named"));
    connect.headers = Some(HeaderMap::from([(String::from("x"), text("old"))]));
    connect.extra_headers = Some(HeaderMap::from([
        (String::from("x"), text("new")),
        (String::from("y"), text("test")),
    ]));
    connect.metadata = Some(IndexMap::from([(
        String::from("guardrails"),
        Json::List(vec![text("first")]),
    )]));
    connect.guardrails = Some(Json::List(vec![text("second")]));
    connect.litellm_metadata = Some(IndexMap::from([
        (
            String::from("guardrails"),
            Json::List(vec![text("third")]),
        ),
        (String::from("trace"), text("t1")),
    ]));
    let litellm_params = empty_litellm_params();
    let query = IndexMap::from([
        (String::from("model"), String::from("alias")),
        (String::from("intent"), String::from("transcription")),
    ]);
    let call = RealtimeConnectCall {
        model: "gemini/gemini-2.0-flash-live-001",
        params: &connect,
        query_params: Some(&query),
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (logging, result) = run(&dispatch, &RealtimeConnectionSettings::default(), &call).await;
    let RealtimeConnection::Provider(plan) = result.unwrap() else {
        panic!("a gemini model must take the generic provider-config branch");
    };
    assert_eq!(dispatch.tags(), vec![RealtimeConfigTag::Gemini]);
    assert_eq!(plan.transport.api_base.as_deref(), Some("https://named.example/v1"));
    assert_eq!(
        plan.transport.api_key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("sk-named")
    );
    assert_eq!(
        plan.transport.headers,
        HeaderMap::from([
            (String::from("x"), text("new")),
            (String::from("y"), text("test")),
        ])
    );
    assert_eq!(
        plan.transport.query_params.as_ref().and_then(|query| query.get("model")),
        Some(&String::from("gemini-2.0-flash-live-001"))
    );
    assert_eq!(
        plan.transport.query_params.as_ref().and_then(|query| query.get("intent")),
        Some(&String::from("transcription"))
    );
    assert_eq!(query.get("model").map(String::as_str), Some("alias"));
    assert_eq!(
        plan.transport.litellm_metadata.get("guardrails"),
        Some(&Json::List(vec![text("first")]))
    );
    assert_eq!(plan.transport.litellm_metadata.get("trace"), Some(&text("t1")));
    assert_eq!(logging.model, "gemini-2.0-flash-live-001");
    assert_eq!(
        logging.litellm_params.get("_arealtime"),
        Some(&Json::Bool(true))
    );
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("gemini"));
    assert!(logging.optional_params.is_empty());
}

#[tokio::test]
async fn realtime_generic_dispatch_keeps_guardrails_and_reaches_meta() {
    let dispatch = Recorder::default();
    let mut connect = params();
    connect.metadata = Some(IndexMap::from([(
        String::from("guardrails"),
        Json::List(Vec::new()),
    )]));
    connect.guardrails = Some(Json::List(Vec::new()));
    connect.litellm_metadata = Some(IndexMap::from([(
        String::from("guardrails"),
        Json::List(vec![text("third")]),
    )]));
    let litellm_params = empty_litellm_params();
    let call = RealtimeConnectCall {
        model: "meta/muse-1",
        params: &connect,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &RealtimeConnectionSettings::default(), &call).await;
    let RealtimeConnection::Provider(plan) = result.unwrap() else {
        panic!("a meta model must take the generic provider-config branch");
    };
    assert_eq!(dispatch.tags(), vec![RealtimeConfigTag::Meta]);
    assert_eq!(
        plan.transport.litellm_metadata.get("guardrails"),
        Some(&Json::List(vec![text("third")]))
    );
    assert_eq!(plan.transport.api_base, None);
}

#[tokio::test]
async fn realtime_generic_dispatch_rejects_an_unsupported_model() {
    let dispatch = Recorder::default();
    let connect = params();
    let litellm_params = empty_litellm_params();
    let call = RealtimeConnectCall {
        model: "anthropic/claude-x",
        params: &connect,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &RealtimeConnectionSettings::default(), &call).await;
    let error = result.unwrap_err();
    assert_eq!(error.to_string(), "Unsupported model: claude-x");
    assert!(dispatch.tags().is_empty());
}

#[tokio::test]
async fn realtime_openai_and_azure_credentials() {
    let settings = RealtimeConnectionSettings {
        api_base: Some(String::from("https://global.example")),
        api_key: Some(secret("sk-global")),
        openai_key: Some(secret("sk-openai-global")),
        azure_api_base: Some(String::from("https://azure.example")),
        azure_api_key: Some(secret("azure-key")),
        openai_api_key: Some(secret("openai-key")),
        ..RealtimeConnectionSettings::default()
    };
    let dispatch = Recorder::default();
    let connect = params();
    let litellm_params = empty_litellm_params();
    let query = IndexMap::from([(String::from("intent"), String::from("transcription"))]);
    let call = RealtimeConnectCall {
        model: "azure/my-deployment",
        params: &connect,
        query_params: Some(&query),
        client: None,
        client_sent_beta: true,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &settings, &call).await;
    let RealtimeConnection::Azure(plan) = result.unwrap() else {
        panic!("an azure model must take the azure branch");
    };
    assert_eq!(plan.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(plan.api_key.as_ref().map(|key| key.expose_secret().as_str()), Some("sk-global"));
    assert_eq!(plan.api_version, "2024-10-01-preview");
    assert_eq!(plan.protocol, "GA");
    assert_eq!(plan.azure_ad_token, None);
    assert!(dispatch.ad_requests.lock().unwrap_or_else(PoisonError::into_inner).is_empty());
    assert_eq!(
        plan.query_params.as_ref().and_then(|query| query.get("intent")),
        Some(&Some(String::from("transcription")))
    );

    let named = RealtimeConnectParams {
        api_base: Some(String::from("https://named.example")),
        api_version: Some(String::from("2025-01-01")),
        realtime_protocol: Some(String::from("beta")),
        ..RealtimeConnectParams::default()
    };
    let call = RealtimeConnectCall {
        model: "azure_ai/my-deployment",
        params: &named,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &settings, &call).await;
    let RealtimeConnection::Azure(plan) = result.unwrap() else {
        panic!("an azure_ai model must take the azure branch");
    };
    assert_eq!(plan.api_base.as_deref(), Some("https://named.example"));
    assert_eq!(plan.api_version, "2025-01-01");
    assert_eq!(plan.protocol, "beta");

    let no_keys = RealtimeConnectionSettings {
        azure_api_base: Some(String::from("https://azure.example")),
        ..RealtimeConnectionSettings::default()
    };
    let ad_token = RealtimeConnectParams {
        azure_ad_token: Some(secret("named-ad")),
        tenant_id: Some(String::from("tenant-1")),
        ..RealtimeConnectParams::default()
    };
    let dispatch = Recorder::with_ad_token(Some(secret("resolved-ad")));
    let call = RealtimeConnectCall {
        model: "azure/my-deployment",
        params: &ad_token,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &no_keys, &call).await;
    let RealtimeConnection::Azure(plan) = result.unwrap() else {
        panic!("an azure model must take the azure branch");
    };
    assert_eq!(plan.api_base.as_deref(), Some("https://azure.example"));
    assert_eq!(plan.api_key, None);
    assert_eq!(
        plan.azure_ad_token.as_ref().map(|token| token.expose_secret().as_str()),
        Some("resolved-ad")
    );
    assert_eq!(
        *dispatch.ad_requests.lock().unwrap_or_else(PoisonError::into_inner),
        vec![(Some(String::from("named-ad")), Some(String::from("tenant-1")))]
    );

    let openai = RealtimeConnectionSettings {
        api_key: None,
        openai_key: Some(secret("sk-openai-global")),
        ..RealtimeConnectionSettings::default()
    };
    let connect = params();
    let call = RealtimeConnectCall {
        model: "openai/gpt-realtime",
        params: &connect,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &openai, &call).await;
    let RealtimeConnection::OpenAi(plan) = result.unwrap() else {
        panic!("an openai model must take the openai branch");
    };
    assert_eq!(plan.api_base, "https://api.openai.com/");
    assert_eq!(
        plan.api_key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("sk-openai-global")
    );
}

#[tokio::test]
async fn realtime_bedrock_and_xai_arguments() {
    let dispatch = Recorder::default();
    let litellm_params = empty_litellm_params();
    let connect = RealtimeConnectParams {
        api_base: Some(String::from("https://bedrock.example")),
        api_key: Some(secret("bedrock-key")),
        timeout: Some(30.0_f64),
        aws_region_name: Some(String::from("us-east-1")),
        aws_access_key_id: Some(secret("access")),
        aws_secret_access_key: Some(secret("secret-access")),
        aws_session_token: Some(secret("session")),
        aws_role_name: Some(String::from("role")),
        aws_session_name: Some(String::from("session-name")),
        aws_profile_name: Some(String::from("profile")),
        aws_web_identity_token: Some(secret("web-identity")),
        aws_sts_endpoint: Some(String::from("https://sts.example")),
        aws_bedrock_runtime_endpoint: Some(String::from("https://runtime.example")),
        aws_external_id: Some(String::from("external")),
        aws_session_tags: Some(text("tags")),
        ..RealtimeConnectParams::default()
    };
    let call = RealtimeConnectCall {
        model: "bedrock/amazon.nova-sonic-v1:0",
        params: &connect,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &RealtimeConnectionSettings::default(), &call).await;
    let RealtimeConnection::Bedrock(plan) = result.unwrap() else {
        panic!("a bedrock model must take the bedrock branch");
    };
    assert_eq!(plan.api_base.as_deref(), Some("https://bedrock.example"));
    assert_eq!(plan.api_key.as_ref().map(|key| key.expose_secret().as_str()), Some("bedrock-key"));
    assert_eq!(plan.timeout, Some(30.0_f64));
    assert_eq!(plan.aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(plan.aws_access_key_id.as_ref().map(|key| key.expose_secret().as_str()), Some("access"));
    assert_eq!(
        plan.aws_secret_access_key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("secret-access")
    );
    assert_eq!(plan.aws_session_token.as_ref().map(|key| key.expose_secret().as_str()), Some("session"));
    assert_eq!(plan.aws_role_name.as_deref(), Some("role"));
    assert_eq!(plan.aws_session_name.as_deref(), Some("session-name"));
    assert_eq!(plan.aws_profile_name.as_deref(), Some("profile"));
    assert_eq!(
        plan.aws_web_identity_token.as_ref().map(|key| key.expose_secret().as_str()),
        Some("web-identity")
    );
    assert_eq!(plan.aws_sts_endpoint.as_deref(), Some("https://sts.example"));
    assert_eq!(plan.aws_bedrock_runtime_endpoint.as_deref(), Some("https://runtime.example"));
    assert_eq!(plan.aws_external_id.as_deref(), Some("external"));
    assert_eq!(plan.aws_session_tags, Some(text("tags")));

    let globals = RealtimeConnectionSettings {
        api_base: Some(String::from("https://global.example")),
        api_key: Some(secret("sk-global")),
        ..RealtimeConnectionSettings::default()
    };
    let connect = RealtimeConnectParams {
        generic_api_base: Some(String::from("https://generic.example")),
        ..RealtimeConnectParams::default()
    };
    let call = RealtimeConnectCall {
        model: "bedrock/amazon.nova-sonic-v1:0",
        params: &connect,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &globals, &call).await;
    let RealtimeConnection::Bedrock(plan) = result.unwrap() else {
        panic!("a bedrock model must take the bedrock branch");
    };
    assert_eq!(plan.api_base, None);
    assert_eq!(plan.api_key, None);

    let xai = RealtimeConnectionSettings {
        api_base: Some(String::from("https://global.example")),
        api_key: Some(secret("sk-global")),
        xai_key: Some(secret("xai-key")),
        xai_api_base: Some(String::from("https://xai-env.example")),
        xai_api_key: Some(secret("xai-env-key")),
        ..RealtimeConnectionSettings::default()
    };
    let connect = RealtimeConnectParams {
        api_base: Some(String::from("https://named.example")),
        ..RealtimeConnectParams::default()
    };
    let call = RealtimeConnectCall {
        model: "xai/grok-4",
        params: &connect,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &xai, &call).await;
    let RealtimeConnection::Xai(plan) = result.unwrap() else {
        panic!("an xai model must take the xai branch");
    };
    assert_eq!(plan.api_base, "https://named.example");
    assert_eq!(plan.api_key.as_ref().map(|key| key.expose_secret().as_str()), Some("xai-key"));

    let connect = params();
    let call = RealtimeConnectCall {
        model: "xai/grok-4",
        params: &connect,
        query_params: None,
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &xai, &call).await;
    let RealtimeConnection::Xai(plan) = result.unwrap() else {
        panic!("an xai model must take the xai branch");
    };
    assert_eq!(plan.api_base, "https://api.x.ai/v1");
}

#[tokio::test]
async fn realtime_vertex_credentials_and_refresh() {
    let tokens = ScriptedTokens::with(vec![
        Ok((secret("token-a"), String::from("resolved-project"))),
        Ok((secret("token-b"), String::from("ignored-project"))),
    ]);
    let dispatch = Recorder::with_tokens(Arc::clone(&tokens));
    let settings = RealtimeConnectionSettings {
        vertex_project: Some(String::from("global-project")),
        vertex_location: Some(String::from("global-region")),
        vertexai_project: Some(String::from("env-project")),
        vertexai_location: Some(String::from("env-region")),
        vertexai_credentials: Some(text("env-credentials")),
        ..RealtimeConnectionSettings::default()
    };
    let named_credentials = Json::Object(IndexMap::from([(
        String::from("type"),
        text("service_account"),
    )]));
    let connect = RealtimeConnectParams {
        vertex_credentials: Some(named_credentials.clone()),
        vertex_project: Some(String::from("named-project")),
        vertex_location: Some(String::from("")),
        vertex_ai_location: Some(String::from("ai-location")),
        timeout: Some(4.0_f64),
        ..RealtimeConnectParams::default()
    };
    let litellm_params = empty_litellm_params();
    let query = IndexMap::from([(String::from("model"), String::from("gemini-live"))]);
    let call = RealtimeConnectCall {
        model: "vertex_ai/gemini-live",
        params: &connect,
        query_params: Some(&query),
        client: None,
        client_sent_beta: false,
        litellm_params: &litellm_params,
    };
    let (_, result) = run(&dispatch, &settings, &call).await;
    let RealtimeConnection::Vertex(plan) = result.unwrap() else {
        panic!("a vertex_ai model must take the vertex branch");
    };
    assert_eq!(settings.credential_timeout_seconds, 20.0_f64);
    assert_eq!(plan.access_token.expose_secret(), "token-a");
    assert_eq!(plan.project, "resolved-project");
    assert_eq!(plan.location.as_deref(), Some("ai-location"));
    assert_eq!(plan.transport.api_base, None);
    assert_eq!(plan.transport.api_key, None);
    assert_eq!(plan.transport.timeout, Some(4.0_f64));
    assert_eq!(
        plan.transport.query_params.as_ref().and_then(|query| query.get("model")),
        Some(&String::from("gemini-live"))
    );
    assert_eq!(
        *tokens.calls.lock().unwrap_or_else(PoisonError::into_inner),
        vec![(Some(named_credentials), Some(String::from("named-project")))]
    );
    let refreshed = plan.refresh.resolve_access_token().await.unwrap();
    assert_eq!(refreshed.expose_secret(), "token-b");
    let calls = tokens.calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].1.as_deref(), Some("resolved-project"));

    let timed_out = resolve_vertex_access_token_bounded(
        tokens.as_ref(),
        None,
        None,
        0.0_f64,
    )
    .await
    .unwrap_err();
    assert_eq!(
        timed_out.to_string(),
        "Vertex AI realtime: timed out fetching Google OAuth access token after 0.0s; check network egress from the proxy to the OAuth token endpoint (oauth2.googleapis.com)"
    );
    let failures = ScriptedTokens::with(vec![Err(VertexTokenError {
        message: String::from("boom"),
    })]);
    let failed = resolve_vertex_access_token_bounded(failures.as_ref(), None, None, 20.0_f64)
        .await
        .unwrap_err();
    assert_eq!(failed.to_string(), "boom");
}
