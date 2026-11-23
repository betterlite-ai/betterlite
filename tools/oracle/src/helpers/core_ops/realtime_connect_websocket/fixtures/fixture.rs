use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::resource_config_selection::realtime::RealtimeConfigTag;
use betterlite_core::api::realtime_connect::token::{
    VertexAccessTokenSource, VertexTokenError, VertexTokenFuture,
};
use betterlite_core::api::realtime_connect::{
    AzureAdTokenRequest, RealtimeConnectDispatch, RealtimeConnectError,
};
use betterlite_core::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use betterlite_core::api::realtime_contracts::error::{HeaderMap, WebSocketError};
use betterlite_core::api::realtime_contracts::events::{
    RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use betterlite_core::api::realtime_contracts::transport::{RealtimeBackend, RealtimeMessage};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde::Deserialize;

const LOG_INIT: &str = r#"{"model":"realtime","call_type":"realtime","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenSetup {
    #[serde(default)]
    pub(crate) tokens: Vec<String>,
    #[serde(default)]
    pub(crate) projects: Vec<String>,
}

pub(crate) struct TestBackend;

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

pub(crate) struct TestConfig;

#[async_trait]
impl BaseRealtimeConfig for TestConfig {
    type Error = io::Error;
    type Backend = TestBackend;

    fn validate_environment(&self, environment: &Environment<'_>) -> Result<HeaderMap, io::Error> {
        Ok(environment.headers.clone())
    }

    fn transform_realtime_request(
        &self,
        _message: &str,
        _model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, io::Error> {
        Ok(Vec::new())
    }

    fn transform_realtime_response(
        &self,
        _message: &RealtimeMessage,
        _model: &str,
        _input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, io::Error> {
        Err(io::Error::other("unused"))
    }
}

type TokenAnswer = Result<(SecretString, String), VertexTokenError>;
type VertexToken = Arc<dyn VertexAccessTokenSource<Error = io::Error>>;
type FixtureError = RealtimeConnectError<io::Error>;

#[derive(Default)]
pub(crate) struct ScriptedTokens {
    answers: Mutex<VecDeque<TokenAnswer>>,
    pub(crate) calls: Mutex<Vec<(Option<Json>, Option<String>)>>,
}

impl VertexAccessTokenSource for ScriptedTokens {
    type Error = io::Error;

    fn resolve<'source>(
        &'source self,
        credentials: Option<&'source Json>,
        project_id: Option<&'source str>,
    ) -> VertexTokenFuture<'source> {
        Box::pin(async move {
            let call = (credentials.cloned(), project_id.map(str::to_owned));
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(call);
            self.answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_else(|| {
                    Err(VertexTokenError {
                        message: String::from("no token"),
                    })
                })
        })
    }
}

pub(crate) struct Fixture {
    pub(crate) tokens: VertexToken,
}

impl RealtimeConnectDispatch for Fixture {
    type Error = io::Error;
    type ProviderConfig = TestConfig;

    fn provider_config(&self, _tag: RealtimeConfigTag) -> Result<Arc<TestConfig>, FixtureError> {
        Ok(Arc::new(TestConfig))
    }

    fn vertex_access_tokens(&self) -> Result<VertexToken, FixtureError> {
        Ok(Arc::clone(&self.tokens))
    }

    fn azure_ad_token(
        &self,
        request: AzureAdTokenRequest<'_>,
    ) -> Result<Option<SecretString>, FixtureError> {
        Ok(request.azure_ad_token.cloned())
    }
}

pub(crate) fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_str(LOG_INIT).or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
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
    .or_value_error()
}

pub(crate) fn scripted_tokens(setup: Option<&TokenSetup>) -> Arc<ScriptedTokens> {
    let tokens = ScriptedTokens::default();
    if let Some(scripted) = setup {
        let mut answers = tokens
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (index, token) in scripted.tokens.iter().enumerate() {
            let project = scripted
                .projects
                .get(index)
                .cloned()
                .unwrap_or_else(|| String::from("resolved-project"));
            answers.push_back(Ok((SecretString::from(token.clone()), project)));
        }
    }
    Arc::new(tokens)
}
