use std::mem::take;
use std::sync::Mutex;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use tokio::runtime::Builder;

use crate::chat_call::{CallStyle, ChatCall, ChatClients, ChatDeps, ChatOutcome, chat_call};
use crate::credentials::AnthropicSettings;
use crate::response::wire::ResponseEnvelope;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::client_kind::SuppliedClient;

use super::fixture::{
    API_BASE, CALLER_KEY, CallError, FixtureConfig, FixtureCounter, FixtureFailure, FixtureIds,
    FixtureReply, FixtureTransport, MODEL, NOW, Recorded, RecordingLogger, ResponseRecord, Silent,
    StreamLoggerCall, TIMEOUT, Trip, logging_env, logging_state, messages, redactor,
    settings_with_key,
};

#[derive(Clone)]
pub(super) struct Setup {
    pub(super) style: CallStyle,
    pub(super) supplied_kind: ClientKind,
    pub(super) supplied: bool,
    pub(super) body: IndexMap<String, Json>,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) litellm_params: IndexMap<String, Json>,
    pub(super) caller_headers: IndexMap<String, String>,
    pub(super) caller_key: Option<&'static str>,
    pub(super) settings_key: Option<&'static str>,
    pub(super) api_base: &'static str,
    pub(super) reply: FixtureReply,
    pub(super) trip: Trip,
    pub(super) transform_failure: Option<&'static str>,
    pub(super) filter_failure: Option<&'static str>,
}

impl Setup {
    // The setup a chat-call test starts from; each case names only the fields it changes
    pub(super) fn chat() -> Self {
        Self {
            style: CallStyle::Sync,
            supplied_kind: ClientKind::Asynchronous,
            supplied: true,
            body: body(),
            optional_params: IndexMap::new(),
            litellm_params: IndexMap::new(),
            caller_headers: caller_headers(),
            caller_key: Some(CALLER_KEY),
            settings_key: None,
            api_base: API_BASE,
            reply: FixtureReply::text("hello"),
            trip: Trip::Reply,
            transform_failure: None,
            filter_failure: None,
        }
    }
}

pub(super) fn body() -> IndexMap<String, Json> {
    IndexMap::from([
        ("model".to_owned(), Json::Text(MODEL.to_owned())),
        ("max_tokens".to_owned(), Json::Number(Number::from(64))),
        (
            "anthropic_beta".to_owned(),
            Json::List(vec![
                Json::Text("keep".to_owned()),
                Json::Text("drop".to_owned()),
            ]),
        ),
    ])
}

pub(super) fn caller_headers() -> IndexMap<String, String> {
    IndexMap::from([("anthropic-beta".to_owned(), "keep,drop".to_owned())])
}

// The converter state B689 builds from a stream call, copied out so a case can read it after opening
pub(super) struct ConverterState {
    pub(super) json_mode: Option<bool>,
    pub(super) speed: Option<String>,
    pub(super) tool_name_reverse_map: IndexMap<String, String>,
}

pub(super) enum Outcome {
    Reply(Box<ResponseEnvelope>),
    Embedding,
    Stream {
        model: String,
        provider: &'static str,
        headers: IndexMap<String, String>,
        options: Box<ConverterState>,
    },
}

pub(super) struct Run {
    pub(super) outcome: Result<Outcome, Box<CallError>>,
    pub(super) posts_supplied: usize,
    pub(super) posts_pooled: usize,
    pub(super) posted_bodies: Vec<IndexMap<String, Json>>,
    pub(super) posted_headers: Vec<HeaderMap>,
    pub(super) transforms: Vec<Recorded>,
    pub(super) calls: Vec<String>,
    pub(super) responses: Vec<ResponseRecord>,
    pub(super) logger_calls: Vec<StreamLoggerCall>,
    pub(super) details: IndexMap<String, Json>,
}

impl Run {
    pub(super) fn error(&self) -> &CallError {
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
        python_json::dumps(&details)
    }
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
    let settings = settings_of(setup);
    let caller_key = setup
        .caller_key
        .map(|key| SecretString::from(key.to_owned()));
    let call = call_of(setup, caller_key.as_ref(), &settings);
    let clients = ChatClients {
        supplied: setup.supplied.then_some(SuppliedClient {
            kind: setup.supplied_kind,
            transport: &supplied,
        }),
        pooled: &pooled,
    };
    let deps = deps_of(&config, clients, &env, &mut state, &logger);
    let runtime = Builder::new_current_thread().build().unwrap();
    let result = runtime.block_on(chat_call(call, deps));
    let outcome = outcome_of(result);
    let (posted_bodies, posted_headers) = posts(&supplied, &pooled);
    let (transforms, calls, logger_calls) = recorded(&config, &logger);
    let responses = take(&mut *config.responses.lock().unwrap());
    Run {
        outcome,
        posts_supplied: supplied.posts(),
        posts_pooled: pooled.posts(),
        posted_bodies,
        posted_headers,
        transforms,
        calls,
        responses,
        logger_calls,
        details: state.model_call_details.clone(),
    }
}

pub(super) fn fixture_config(setup: &Setup) -> FixtureConfig {
    FixtureConfig {
        body: setup.body.clone(),
        transform_failure: setup.transform_failure.map(str::to_owned),
        filter_failure: setup.filter_failure.map(str::to_owned),
        transforms: Mutex::new(Vec::new()),
        calls: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    }
}

pub(super) fn settings_of(setup: &Setup) -> AnthropicSettings {
    settings_with_key(
        setup
            .settings_key
            .map(|key| SecretString::from(key.to_owned())),
    )
}

pub(super) fn call_of<'setup>(
    setup: &'setup Setup,
    api_key: Option<&'setup SecretString>,
    settings: &'setup AnthropicSettings,
) -> ChatCall<'setup> {
    ChatCall {
        model: MODEL,
        custom_llm_provider: "anthropic",
        messages: messages(),
        optional_params: setup.optional_params.clone(),
        litellm_params: setup.litellm_params.clone(),
        caller_headers: setup.caller_headers.clone(),
        api_base: setup.api_base,
        api_key,
        settings,
        timeout: TIMEOUT,
        style: setup.style,
        effort_used: false,
        now: NOW,
        disable_add_prefix_to_prompt: false,
        model_response: ResponseEnvelope::new("chatcmpl-1".to_owned()),
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

pub(super) fn outcome_of(
    result: Result<ChatOutcome<FixtureFailure, FixtureCounter>, CallError>,
) -> Result<Outcome, Box<CallError>> {
    match result {
        Ok(ChatOutcome::Reply(envelope)) => Ok(Outcome::Reply(envelope)),
        Ok(ChatOutcome::Embedding) => Ok(Outcome::Embedding),
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
        Err(error) => Err(Box::new(error)),
    }
}

fn posts(
    supplied: &FixtureTransport,
    pooled: &FixtureTransport,
) -> (Vec<IndexMap<String, Json>>, Vec<HeaderMap>) {
    let mut posted_bodies = supplied.posted_bodies();
    posted_bodies.extend(pooled.posted_bodies());
    let mut posted_headers = supplied.posted_headers();
    posted_headers.extend(pooled.posted_headers());
    (posted_bodies, posted_headers)
}

fn recorded(
    config: &FixtureConfig,
    logger: &RecordingLogger,
) -> (Vec<Recorded>, Vec<String>, Vec<StreamLoggerCall>) {
    let transforms = take(&mut *config.transforms.lock().unwrap());
    let calls = take(&mut *config.calls.lock().unwrap());
    let logger_calls = take(&mut *logger.calls.lock().unwrap());
    (transforms, calls, logger_calls)
}
