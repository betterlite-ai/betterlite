use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error as StdError;
use std::fmt;
use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::api::realtime_connect::{
    DEFAULT_CREDENTIAL_TIMEOUT_SECONDS, RealtimeConnectionSettings,
};
use betterlite_context::resource_config_selection::realtime::RealtimeConfigTag;
use betterlite_core::api::realtime_connect::{
    AzureAdTokenRequest, AzureConnection, BedrockConnection, OpenAiConnection,
    ProviderConfigConnection, RealtimeConnectCall, RealtimeConnectDispatch, RealtimeConnectError,
    RealtimeConnectParams, RealtimeConnection, VertexAccessTokenSource, VertexConnection,
    VertexTokenError, VertexTokenFuture, XaiConnection, arealtime,
};
use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_core::api::vertex_chirp::target::AccessTokenResolver;
use betterlite_core::dispatch::error::DispatchError;
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
use betterlite_openai::realtime::url::QueryParams;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::time::error::Elapsed;
use tokio::time::timeout;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        drive(input)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsInput {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) openai_key: Option<String>,
    pub(crate) xai_key: Option<String>,
    pub(crate) vertex_project: Option<String>,
    pub(crate) vertex_location: Option<String>,
    pub(crate) azure_api_base: Option<String>,
    pub(crate) azure_api_key: Option<String>,
    pub(crate) openai_api_key: Option<String>,
    pub(crate) xai_api_base: Option<String>,
    pub(crate) xai_api_key: Option<String>,
    pub(crate) vertexai_credentials: Option<Json>,
    pub(crate) vertexai_project: Option<String>,
    pub(crate) vertexai_location: Option<String>,
    pub(crate) azure_realtime_protocol: Option<String>,
    pub(crate) credential_timeout_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenSetup {
    pub(crate) tokens: Vec<String>,
    pub(crate) projects: Vec<String>,
    #[serde(default)]
    pub(crate) pending: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) params: RealtimeConnectParams,
    #[serde(default)]
    pub(crate) query_params: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub(crate) client_sent_beta: bool,
    #[serde(default)]
    pub(crate) settings: SettingsInput,
    #[serde(default)]
    pub(crate) token: Option<TokenSetup>,
    #[serde(default)]
    pub(crate) ad_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) scenarios: Vec<Scenario>,
}

#[derive(Debug)]
struct TestError;

impl fmt::Display for TestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the recorded operation failed")
    }
}

impl StdError for TestError {}

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
type TokenCall = (Option<Json>, Option<String>);

struct ScriptedTokens {
    answers: Mutex<VecDeque<TokenAnswer>>,
    calls: Mutex<Vec<TokenCall>>,
    pending: bool,
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
            if self.pending {
                return pending::<TokenAnswer>().await;
            }
            self.answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| {
                    Err(VertexTokenError {
                        message: String::from("the script has no token left"),
                    })
                })
        })
    }
}

#[derive(Default)]
struct ScenarioDispatch {
    tags: Mutex<Vec<RealtimeConfigTag>>,
    ad_requests: Mutex<Vec<(Option<String>, Option<String>)>>,
    ad_reply: Mutex<Option<Option<SecretString>>>,
    tokens: Mutex<Option<Arc<ScriptedTokens>>>,
}

impl RealtimeConnectDispatch for ScenarioDispatch {
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

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("RuntimeError"),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

fn drive(input: &Input) -> HelperResult {
    let runtime = RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error(error.to_string()))?;
    let mut observations = Vec::new();
    for scenario in &input.scenarios {
        observations.push(runtime.block_on(run_scenario(scenario))?);
    }
    Ok(json!({ "scenarios": observations }))
}

struct PreparedScenario {
    dispatch: ScenarioDispatch,
    tokens: Option<Arc<ScriptedTokens>>,
}

fn prepare_scenario(scenario: &Scenario) -> PreparedScenario {
    let dispatch = ScenarioDispatch::default();
    *dispatch
        .ad_reply
        .lock()
        .unwrap_or_else(PoisonError::into_inner) =
        Some(scenario.ad_token.as_deref().map(SecretString::from));
    let tokens = scenario.token.as_ref().map(|setup| {
        let mut answers = VecDeque::new();
        for (index, token) in setup.tokens.iter().enumerate() {
            let project = setup
                .projects
                .get(index)
                .cloned()
                .unwrap_or_else(|| String::from("resolved-project"));
            answers.push_back(Ok((SecretString::from(token.clone()), project)));
        }
        let scripted = Arc::new(ScriptedTokens {
            answers: Mutex::new(answers),
            calls: Mutex::new(Vec::new()),
            pending: setup.pending,
        });
        *dispatch
            .tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&scripted));
        scripted
    });
    PreparedScenario { dispatch, tokens }
}

fn settings_from(input: &SettingsInput) -> RealtimeConnectionSettings {
    RealtimeConnectionSettings {
        api_base: input.api_base.clone(),
        api_key: input.api_key.as_deref().map(SecretString::from),
        openai_key: input.openai_key.as_deref().map(SecretString::from),
        xai_key: input.xai_key.as_deref().map(SecretString::from),
        vertex_project: input.vertex_project.clone(),
        vertex_location: input.vertex_location.clone(),
        azure_api_base: input.azure_api_base.clone(),
        azure_api_key: input.azure_api_key.as_deref().map(SecretString::from),
        openai_api_key: input.openai_api_key.as_deref().map(SecretString::from),
        xai_api_base: input.xai_api_base.clone(),
        xai_api_key: input.xai_api_key.as_deref().map(SecretString::from),
        vertexai_credentials: input.vertexai_credentials.clone(),
        vertexai_project: input.vertexai_project.clone(),
        vertexai_location: input.vertexai_location.clone(),
        azure_realtime_protocol: input.azure_realtime_protocol.clone(),
        credential_timeout_seconds: input
            .credential_timeout_seconds
            .unwrap_or(DEFAULT_CREDENTIAL_TIMEOUT_SECONDS),
    }
}

fn logging_state() -> Result<LoggingState, String> {
    let init: LoggingInit = serde_json::from_value(json!({
        "model": "realtime",
        "call_type": "realtime",
        "start_time": "2026-01-01T00:00:00+00:00",
        "litellm_call_id": "call-1",
        "function_id": "fn-1",
        "litellm_trace_id": "trace-1",
    }))
    .map_err(|error| error.to_string())?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| error.to_string())?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default(),
    )
    .map_err(|error| error.to_string())
}

async fn run_scenario(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let settings = settings_from(&scenario.settings);
    let prepared = prepare_scenario(scenario);
    let mut logging = logging_state().map_err(helper_error)?;
    let connected = execute(scenario, &settings, &prepared.dispatch, &mut logging)
        .await
        .map_err(helper_error)?;
    let refreshed = refresh_token(&connected).await;
    Ok(observation(
        &prepared.dispatch,
        prepared.tokens.as_ref(),
        &logging,
        &connected,
        refreshed,
    ))
}

async fn execute(
    scenario: &Scenario,
    settings: &RealtimeConnectionSettings,
    dispatch: &ScenarioDispatch,
    logging: &mut LoggingState,
) -> Result<Result<RealtimeConnection<TestConfig>, RealtimeConnectError<TestError>>, String> {
    let litellm_params = IndexMap::new();
    let call = RealtimeConnectCall {
        model: &scenario.model,
        params: &scenario.params,
        query_params: scenario.query_params.as_ref(),
        client: None,
        client_sent_beta: scenario.client_sent_beta,
        litellm_params: &litellm_params,
    };
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
    timeout(
        Duration::from_secs(120),
        arealtime(dispatch, settings, logging, &resolution, &call),
    )
    .await
    .map_err(|error: Elapsed| error.to_string())
}

async fn refresh_token(
    result: &Result<RealtimeConnection<TestConfig>, RealtimeConnectError<TestError>>,
) -> Option<Option<String>> {
    let RealtimeConnection::Vertex(plan) = result.as_ref().ok()? else {
        return None;
    };
    let resolved = timeout(
        Duration::from_secs(120),
        plan.refresh.resolve_access_token(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .map(|token| token.expose_secret().to_owned());
    Some(resolved)
}

fn observation(
    dispatch: &ScenarioDispatch,
    tokens: Option<&Arc<ScriptedTokens>>,
    logging: &LoggingState,
    result: &Result<RealtimeConnection<TestConfig>, RealtimeConnectError<TestError>>,
    refreshed: Option<Option<String>>,
) -> Value {
    match result {
        Ok(RealtimeConnection::Provider(plan)) => provider_json(plan, logging, dispatch),
        Ok(RealtimeConnection::Azure(plan)) => azure_json(plan, logging, dispatch),
        Ok(RealtimeConnection::OpenAi(plan)) => openai_json(plan, logging),
        Ok(RealtimeConnection::Bedrock(plan)) => bedrock_json(plan, logging),
        Ok(RealtimeConnection::Xai(plan)) => xai_json(plan, logging),
        Ok(RealtimeConnection::Vertex(plan)) => vertex_json(plan, logging, tokens, refreshed),
        Err(error) => error_json(error, logging),
    }
}

struct Observation<'obs> {
    branch: &'static str,
    model: Option<&'obs str>,
    api_base: Option<&'obs str>,
    api_key: Option<&'obs SecretString>,
    timeout: Option<f64>,
    query: Value,
    metadata: Option<Value>,
    principal: Option<&'obs Json>,
    headers: Option<Value>,
    client_is_none: bool,
    details: Value,
    logging: &'obs LoggingState,
    error: Option<&'obs str>,
}

impl Observation<'_> {
    fn into_value(self) -> Value {
        json!({
            "branch": self.branch,
            "model": self.model,
            "api_base": self.api_base,
            "api_key": plain(self.api_key),
            "timeout": self.timeout,
            "query": self.query,
            "metadata": self.metadata,
            "principal": self.principal,
            "headers": self.headers,
            "client_is_none": self.client_is_none,
            "details": self.details,
            "logging": logging_json(self.logging),
            "error": self.error,
        })
    }
}

fn provider_json(
    plan: &ProviderConfigConnection<TestConfig>,
    logging: &LoggingState,
    dispatch: &ScenarioDispatch,
) -> Value {
    let transport = &plan.transport;
    Observation {
        branch: "provider",
        model: Some(plan.model.as_str()),
        api_base: transport.api_base.as_deref(),
        api_key: transport.api_key.as_ref(),
        timeout: transport.timeout,
        query: query_pairs(transport.query_params.as_ref()),
        metadata: Some(Json::Object(transport.litellm_metadata.clone())),
        principal: transport.user_api_key_dict.as_ref(),
        headers: Some(header_pairs(&transport.headers)),
        client_is_none: transport.client.is_none(),
        details: json!({ "tag": tag_name(dispatch) }),
        logging,
        error: None,
    }
    .into_value()
}

fn azure_json(
    plan: &AzureConnection,
    logging: &LoggingState,
    dispatch: &ScenarioDispatch,
) -> Value {
    Observation {
        branch: "azure",
        model: Some(plan.model.as_str()),
        api_base: plan.api_base.as_deref(),
        api_key: plan.api_key.as_ref(),
        timeout: plan.timeout,
        query: connector_pairs(plan.query_params.as_ref()),
        metadata: Some(Json::Object(plan.litellm_metadata.clone())),
        principal: plan.user_api_key_dict.as_ref(),
        headers: None,
        client_is_none: true,
        details: json!({
            "api_version": plan.api_version.clone(),
            "protocol": plan.protocol.clone(),
            "azure_ad_token": plain(plan.azure_ad_token.as_ref()),
            "ad_lookup": ad_lookup(dispatch),
        }),
        logging,
        error: None,
    }
    .into_value()
}

fn openai_json(plan: &OpenAiConnection, logging: &LoggingState) -> Value {
    Observation {
        branch: "openai",
        model: Some(plan.model.as_str()),
        api_base: Some(plan.api_base.as_str()),
        api_key: plan.api_key.as_ref(),
        timeout: plan.timeout,
        query: connector_pairs(plan.query_params.as_ref()),
        metadata: Some(Json::Object(plan.litellm_metadata.clone())),
        principal: plan.user_api_key_dict.as_ref(),
        headers: None,
        client_is_none: true,
        details: json!({}),
        logging,
        error: None,
    }
    .into_value()
}

fn xai_json(plan: &XaiConnection, logging: &LoggingState) -> Value {
    Observation {
        branch: "xai",
        model: Some(plan.model.as_str()),
        api_base: Some(plan.api_base.as_str()),
        api_key: plan.api_key.as_ref(),
        timeout: plan.timeout,
        query: connector_pairs(plan.query_params.as_ref()),
        metadata: Some(Json::Object(plan.litellm_metadata.clone())),
        principal: plan.user_api_key_dict.as_ref(),
        headers: None,
        client_is_none: true,
        details: json!({}),
        logging,
        error: None,
    }
    .into_value()
}

fn bedrock_json(plan: &BedrockConnection, logging: &LoggingState) -> Value {
    Observation {
        branch: "bedrock",
        model: Some(plan.model.as_str()),
        api_base: plan.api_base.as_deref(),
        api_key: plan.api_key.as_ref(),
        timeout: plan.timeout,
        query: Value::Array(Vec::new()),
        metadata: None,
        principal: None,
        headers: None,
        client_is_none: true,
        details: json!({
            "aws_region_name": plan.aws_region_name.clone(),
            "aws_access_key_id": plain(plan.aws_access_key_id.as_ref()),
            "aws_secret_access_key": plain(plan.aws_secret_access_key.as_ref()),
            "aws_session_token": plain(plan.aws_session_token.as_ref()),
            "aws_role_name": plan.aws_role_name.clone(),
            "aws_session_name": plan.aws_session_name.clone(),
            "aws_profile_name": plan.aws_profile_name.clone(),
            "aws_web_identity_token": plain(plan.aws_web_identity_token.as_ref()),
            "aws_sts_endpoint": plan.aws_sts_endpoint.clone(),
            "aws_bedrock_runtime_endpoint": plan.aws_bedrock_runtime_endpoint.clone(),
            "aws_external_id": plan.aws_external_id.clone(),
            "aws_session_tags": plan.aws_session_tags.clone(),
        }),
        logging,
        error: None,
    }
    .into_value()
}

fn vertex_json(
    plan: &VertexConnection,
    logging: &LoggingState,
    tokens: Option<&Arc<ScriptedTokens>>,
    refreshed: Option<Option<String>>,
) -> Value {
    let transport = &plan.transport;
    Observation {
        branch: "vertex",
        model: Some(plan.model.as_str()),
        api_base: transport.api_base.as_deref(),
        api_key: transport.api_key.as_ref(),
        timeout: transport.timeout,
        query: query_pairs(transport.query_params.as_ref()),
        metadata: Some(Json::Object(transport.litellm_metadata.clone())),
        principal: transport.user_api_key_dict.as_ref(),
        headers: Some(header_pairs(&transport.headers)),
        client_is_none: transport.client.is_none(),
        details: json!({
            "access_token": plan.access_token.expose_secret(),
            "project": plan.project.clone(),
            "location": plan.location.clone(),
            "refresh_token": refreshed.flatten(),
            "token_calls": token_calls(tokens),
        }),
        logging,
        error: None,
    }
    .into_value()
}

fn error_json(error: &RealtimeConnectError<TestError>, logging: &LoggingState) -> Value {
    let message = error.to_string();
    Observation {
        branch: "error",
        model: None,
        api_base: None,
        api_key: None,
        timeout: None,
        query: Value::Array(Vec::new()),
        metadata: None,
        principal: None,
        headers: None,
        client_is_none: true,
        details: json!({}),
        logging,
        error: Some(message.as_str()),
    }
    .into_value()
}

fn logging_json(logging: &LoggingState) -> Value {
    json!({
        "model": logging.model.clone(),
        "user": logging.user.clone(),
        "custom_provider": logging.custom_llm_provider.clone(),
        "arealtime": logging.litellm_params.get("_arealtime") == Some(&Json::Bool(true)),
    })
}

fn plain(secret: Option<&SecretString>) -> Option<String> {
    secret.map(|secret| secret.expose_secret().to_owned())
}

fn tag_name(dispatch: &ScenarioDispatch) -> &'static str {
    match dispatch
        .tags
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
    {
        Some(RealtimeConfigTag::Gemini) => "gemini",
        Some(RealtimeConfigTag::Meta) => "meta",
        None => "none",
    }
}

fn ad_lookup(dispatch: &ScenarioDispatch) -> Value {
    let requests = dispatch
        .ad_requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match requests.first() {
        None => Value::Null,
        Some((token, tenant)) => json!({
            "named_azure_ad_token": token,
            "tenant_id": tenant,
        }),
    }
}

fn token_calls(tokens: Option<&Arc<ScriptedTokens>>) -> Value {
    let Some(tokens) = tokens else {
        return Value::Array(Vec::new());
    };
    let calls = tokens.calls.lock().unwrap_or_else(PoisonError::into_inner);
    Value::Array(
        calls
            .iter()
            .map(|(credentials, project)| json!({ "credentials": credentials, "project": project }))
            .collect(),
    )
}

fn query_pairs(query: Option<&IndexMap<String, String>>) -> Value {
    let Some(query) = query else {
        return Value::Array(Vec::new());
    };
    let mut pairs: Vec<(&str, &str)> = query
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    pairs.sort();
    Value::Array(
        pairs
            .into_iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

fn connector_pairs(query: Option<&QueryParams>) -> Value {
    let Some(query) = query else {
        return Value::Array(Vec::new());
    };
    let mut pairs: Vec<(&str, &str)> = query
        .iter()
        .filter_map(|(name, value)| value.as_deref().map(|value| (name.as_str(), value)))
        .collect();
    pairs.sort();
    Value::Array(
        pairs
            .into_iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}

fn header_pairs(headers: &HeaderMap) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect(),
    )
}
