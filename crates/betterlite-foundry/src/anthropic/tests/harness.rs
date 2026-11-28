use std::convert::Infallible;
use std::mem::take;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_anthropic::chat_call::{ChatClients, ChatDeps, ChatOutcome};
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::client_kind::SuppliedClient;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::anthropic::error::ChatError;
use crate::anthropic::handler::{CompletionResult, completion};
use crate::anthropic::request::{ChatRequest, Style};
use crate::anthropic_messages::environment::{ParamsKey, TokenSource};

use super::support::{
    API_BASE, CALLER_KEY, FixtureConfig, FixtureCounter, FixtureFailure, FixtureIds, FixtureReply,
    FixtureTransport, LoggerCall, MODEL, NOW, Offline, Recorded, RecordingLogger, ResponseRecord,
    Silent, TIMEOUT, Trip, body, logging_env, logging_state, messages, redactor,
};

#[derive(Clone, Copy)]
pub(super) enum KeyShape {
    DictAbsent,
    DictNull,
    Typed(Option<&'static str>),
}

#[derive(Clone)]
pub(super) struct Setup {
    pub(super) style: Style,
    pub(super) supplied_kind: ClientKind,
    pub(super) supplied: bool,
    pub(super) messages: Option<Vec<Json>>,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) litellm_params: IndexMap<String, Json>,
    pub(super) caller_headers: IndexMap<String, String>,
    pub(super) key_shape: KeyShape,
    pub(super) argument_key: Option<&'static str>,
    pub(super) configured_key: Option<&'static str>,
    pub(super) token: Option<&'static str>,
    pub(super) client_secret: Option<&'static str>,
    pub(super) api_base: &'static str,
    pub(super) reply: FixtureReply,
    pub(super) trip: Trip,
    pub(super) transform_body: Option<IndexMap<String, Json>>,
    pub(super) transform_failure: Option<&'static str>,
}

impl Setup {
    // The setup a completion test starts from; each case names only the fields it changes
    pub(super) fn chat() -> Self {
        Self {
            style: Style::Sync,
            supplied_kind: ClientKind::Sync,
            supplied: true,
            messages: None,
            optional_params: IndexMap::new(),
            litellm_params: IndexMap::new(),
            caller_headers: IndexMap::new(),
            key_shape: KeyShape::DictAbsent,
            argument_key: Some(CALLER_KEY),
            configured_key: None,
            token: None,
            client_secret: None,
            api_base: API_BASE,
            reply: FixtureReply::text("hello"),
            trip: Trip::Reply,
            transform_body: Some(body()),
            transform_failure: None,
        }
    }
}

pub(super) struct ConverterState {
    pub(super) json_mode: Option<bool>,
    pub(super) speed: Option<String>,
    pub(super) tool_name_reverse_map: IndexMap<String, String>,
}

pub(super) enum Outcome {
    Reply(Box<ResponseEnvelope>),
    Stream {
        model: String,
        provider: &'static str,
        headers: IndexMap<String, String>,
        options: Box<ConverterState>,
    },
    Embedding,
}

pub(super) type FixtureError = ChatError<FixtureFailure, Infallible>;
pub(super) type CallResult = CompletionResult<FixtureTransport, FixtureCounter>;

pub(super) struct Run {
    pub(super) outcome: Result<Outcome, Box<FixtureError>>,
    pub(super) posts_supplied: usize,
    pub(super) posts_pooled: usize,
    pub(super) posted_bodies: Vec<IndexMap<String, Json>>,
    pub(super) posted_headers: Vec<HeaderMap>,
    pub(super) posted_urls: Vec<String>,
    pub(super) posted_deadlines: Vec<Duration>,
    pub(super) transforms: Vec<Recorded>,
    pub(super) calls: Vec<String>,
    pub(super) responses: Vec<ResponseRecord>,
    pub(super) logger_calls: Vec<LoggerCall>,
    pub(super) details: IndexMap<String, Json>,
}

impl Run {
    pub(super) fn error(&self) -> &FixtureError {
        self.outcome.as_ref().err().unwrap()
    }

    pub(super) fn status(&self) -> Option<u16> {
        let error = self.outcome.as_ref().err()?;
        error.status_code()
    }

    pub(super) fn reply(&self) -> &ResponseEnvelope {
        let Ok(Outcome::Reply(envelope)) = &self.outcome else {
            panic!("the call did not reply");
        };
        envelope
    }

    pub(super) fn stream(
        &self,
    ) -> (
        &str,
        &'static str,
        &IndexMap<String, String>,
        &ConverterState,
    ) {
        let Ok(Outcome::Stream {
            model,
            provider,
            headers,
            options,
        }) = &self.outcome
        else {
            panic!("the call did not stream");
        };
        (model.as_str(), *provider, headers, options.as_ref())
    }

    pub(super) fn detail(&self, path: &[&str]) -> Option<&Json> {
        let (first, rest) = path.split_first()?;
        let mut current = self.details.get(*first)?;
        for name in rest {
            let Json::Object(fields) = current else {
                return None;
            };
            current = fields.get(*name)?;
        }
        Some(current)
    }

    pub(super) fn details_text(&self) -> String {
        let details = Json::Object(self.details.clone());
        dumps(&details)
    }
}

pub(super) struct Keys {
    argument: Option<SecretString>,
    configured: Option<SecretString>,
    params: Option<SecretString>,
}

pub(super) fn execute(setup: &Setup) -> Run {
    let redactor = redactor();
    let sink = Silent;
    let mut state = logging_state(&redactor);
    let env = logging_env(&redactor, &sink);
    let config = fixture_config(setup);
    let supplied = FixtureTransport::new(setup.reply.clone(), setup.trip.clone());
    let pooled = FixtureTransport::new(setup.reply.clone(), setup.trip.clone());
    let logger = RecordingLogger::default();
    let (credentials, keys) = signing_material(setup);
    let request = request_of(setup, &credentials, &keys);
    let clients = ChatClients {
        supplied: setup.supplied.then_some(SuppliedClient {
            kind: setup.supplied_kind,
            transport: &supplied,
        }),
        pooled: &pooled,
    };
    let deps = deps_of(&config, clients, &env, &mut state, &logger);
    let runtime = Builder::new_current_thread().build().unwrap();
    let result: CallResult = runtime.block_on(completion(request, deps));
    let outcome = outcome_of(result);
    let posted = posts(&supplied, &pooled);
    let (transforms, calls, logger_calls) = recorded(&config, &logger);
    let responses = responses_of(&config);
    Run {
        outcome,
        posts_supplied: supplied.posts(),
        posts_pooled: pooled.posts(),
        posted_bodies: posted.bodies,
        posted_headers: posted.headers,
        posted_urls: posted.urls,
        posted_deadlines: posted.deadlines,
        transforms,
        calls,
        responses,
        logger_calls,
        details: state.model_call_details.clone(),
    }
}

pub(super) fn deps_of<'deps>(
    config: &'deps FixtureConfig,
    clients: ChatClients<'deps, FixtureTransport>,
    env: &'deps PrePostEnv<'deps>,
    state: &'deps mut LoggingState,
    logger: &'deps RecordingLogger,
) -> ChatDeps<'deps, FixtureConfig, FixtureTransport, FixtureCounter> {
    ChatDeps {
        config,
        clients,
        state,
        env,
        stream_logger: logger,
        counter: FixtureCounter,
        ids: &FixtureIds,
        max_event_bytes: 0x0010_0000,
        max_error_body_bytes: 0x0010_0000,
    }
}

pub(super) fn fixture_config(setup: &Setup) -> FixtureConfig {
    FixtureConfig {
        body: setup.transform_body.clone(),
        transform_failure: setup.transform_failure.map(str::to_owned),
        transforms: Mutex::new(Vec::new()),
        calls: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    }
}

// The Azure deployment credentials and the caller's key shapes the request builder resolves auth from
pub(super) fn signing_material(setup: &Setup) -> (DeploymentSettings, Keys) {
    let client_secret = setup.client_secret.map(SecretString::from);
    let typed = match setup.key_shape {
        KeyShape::Typed(key) => key.map(secret),
        KeyShape::DictAbsent | KeyShape::DictNull => None,
    };
    let credentials = DeploymentSettings {
        azure_ad_token: setup.token.map(SecretString::from),
        tenant_id: client_secret.as_ref().map(|_| "tenant".to_owned()),
        client_id: client_secret.as_ref().map(|_| "client".to_owned()),
        client_secret,
        azure_scope: "https://cognitiveservices.azure.com/.default".to_owned(),
        azure_username: None,
        azure_password: None,
    };
    let keys = Keys {
        argument: setup.argument_key.map(secret),
        configured: setup.configured_key.map(secret),
        params: typed,
    };
    (credentials, keys)
}

fn secret(text: &'static str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(super) fn request_of<'setup>(
    setup: &'setup Setup,
    credentials: &'setup DeploymentSettings,
    keys: &'setup Keys,
) -> ChatRequest<'setup> {
    let params_key = match setup.key_shape {
        KeyShape::DictAbsent => ParamsKey::DictAbsent,
        KeyShape::DictNull => ParamsKey::DictNull,
        KeyShape::Typed(_) => ParamsKey::Typed(keys.params.as_ref()),
    };
    ChatRequest {
        model: MODEL,
        messages: setup.messages.clone().unwrap_or_else(messages),
        optional_params: setup.optional_params.clone(),
        litellm_params: setup.litellm_params.clone(),
        caller_headers: setup.caller_headers.clone(),
        api_base: setup.api_base,
        params_key,
        argument_key: keys.argument.as_ref(),
        configured_key: keys.configured.as_ref(),
        token: TokenSource {
            credentials,
            refresh_enabled: false,
            provider: None,
            sources: &Offline,
        },
        timeout: TIMEOUT,
        style: setup.style,
        now: NOW,
        disable_add_prefix_to_prompt: false,
        model_response: ResponseEnvelope::new("chatcmpl-1".to_owned()),
    }
}

pub(super) fn outcome_of(result: CallResult) -> Result<Outcome, Box<FixtureError>> {
    match result {
        Ok(ChatOutcome::Reply(envelope)) => Ok(Outcome::Reply(envelope)),
        Ok(ChatOutcome::Stream(wrapped)) => {
            let stream = *wrapped;
            let state = &stream.completion_stream.converter.state;
            let options = ConverterState {
                json_mode: state.json_mode,
                speed: state.speed.clone(),
                tool_name_reverse_map: state.tool_name_reverse_map.clone(),
            };
            Ok(Outcome::Stream {
                model: stream.model,
                provider: stream.custom_llm_provider,
                headers: stream.response_headers,
                options: Box::new(options),
            })
        }
        Ok(ChatOutcome::Embedding) => Ok(Outcome::Embedding),
        Err(error) => Err(Box::new(error)),
    }
}

struct Posted {
    bodies: Vec<IndexMap<String, Json>>,
    headers: Vec<HeaderMap>,
    urls: Vec<String>,
    deadlines: Vec<Duration>,
}

fn posts(supplied: &FixtureTransport, pooled: &FixtureTransport) -> Posted {
    let mut bodies = supplied.posted_bodies();
    bodies.extend(pooled.posted_bodies());
    let mut headers = supplied.posted_headers();
    headers.extend(pooled.posted_headers());
    let mut urls = supplied.posted_urls();
    urls.extend(pooled.posted_urls());
    let mut deadlines = supplied.posted_deadlines();
    deadlines.extend(pooled.posted_deadlines());
    Posted {
        bodies,
        headers,
        urls,
        deadlines,
    }
}

fn responses_of(config: &FixtureConfig) -> Vec<ResponseRecord> {
    take(
        &mut *config
            .responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    )
}

fn recorded(
    config: &FixtureConfig,
    logger: &RecordingLogger,
) -> (Vec<Recorded>, Vec<String>, Vec<LoggerCall>) {
    let transforms = take(
        &mut *config
            .transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let calls = take(&mut *config.calls.lock().unwrap_or_else(PoisonError::into_inner));
    let logger_calls = take(&mut *logger.calls.lock().unwrap_or_else(PoisonError::into_inner));
    (transforms, calls, logger_calls)
}
