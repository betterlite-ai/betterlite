use std::sync::Mutex;

use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::chat_call::transport::{ClientKind, SuppliedChatClient};
use crate::chat_call::{ChatCall, ChatClients, ChatDeps, ChatOutcome, chat_call};
use crate::credentials::AnthropicSettings;
use crate::response::wire::ResponseEnvelope;

use super::fixture::{
    API_BASE, CALLER_KEY, CallError, FixtureConfig, FixtureCounter, FixtureIds, FixtureReply,
    FixtureTransport, MODEL, NOW, Recorded, RecordingLogger, Silent, StreamLoggerCall, TIMEOUT,
    Trip, logging_env, logging_state, messages, redactor,
};

#[derive(Clone)]
pub(super) struct Setup {
    pub(super) acompletion: bool,
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

impl Default for Setup {
    fn default() -> Self {
        Self {
            acompletion: false,
            supplied_kind: ClientKind::Async,
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
        (
            "max_tokens".to_owned(),
            Json::Number(serde_json::Number::from(64)),
        ),
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
    Stream {
        model: String,
        provider: &'static str,
        headers: IndexMap<String, String>,
        options: ConverterState,
    },
}

pub(super) struct Run {
    pub(super) outcome: Result<Outcome, CallError>,
    pub(super) posts_supplied: usize,
    pub(super) posts_pooled: usize,
    pub(super) posted_bodies: Vec<IndexMap<String, Json>>,
    pub(super) posted_headers: Vec<HeaderMap>,
    pub(super) transforms: Vec<Recorded>,
    pub(super) calls: Vec<String>,
    pub(super) logger_calls: Vec<StreamLoggerCall>,
    pub(super) details: IndexMap<String, Json>,
}

impl Run {
    pub(super) fn error(&self) -> &CallError {
        self.outcome.as_ref().err().unwrap()
    }

    pub(super) fn status(&self) -> Option<u16> {
        self.outcome.as_ref().err().map(|error| error.status_code())
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
        (model.as_str(), *provider, headers, options)
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

pub(super) fn execute(setup: Setup) -> Run {
    let redactor: Redactor = redactor();
    let sink = Silent;
    let mut state = logging_state(&redactor);
    let env = logging_env(&redactor, &sink);
    let config = FixtureConfig {
        body: setup.body,
        transform_failure: setup.transform_failure.map(str::to_owned),
        filter_failure: setup.filter_failure.map(str::to_owned),
        transforms: Mutex::new(Vec::new()),
        calls: Mutex::new(Vec::new()),
    };
    let supplied = FixtureTransport::new(setup.reply.clone(), setup.trip.clone());
    let pooled = FixtureTransport::new(setup.reply, setup.trip);
    let logger = RecordingLogger::default();
    let caller_key = setup
        .caller_key
        .map(|key| SecretString::from(key.to_owned()));
    let mut settings = AnthropicSettings::default();
    settings.api_key = setup
        .settings_key
        .map(|key| SecretString::from(key.to_owned()));
    let call = ChatCall {
        model: MODEL,
        custom_llm_provider: "anthropic",
        messages: messages(),
        optional_params: setup.optional_params,
        litellm_params: setup.litellm_params,
        caller_headers: setup.caller_headers,
        api_base: setup.api_base,
        api_key: caller_key.as_ref(),
        settings: &settings,
        timeout: TIMEOUT,
        acompletion: setup.acompletion,
        effort_used: false,
        now: NOW,
        disable_add_prefix_to_prompt: false,
        model_response: ResponseEnvelope::new("chatcmpl-1".to_owned()),
    };
    let entry = SuppliedChatClient {
        kind: setup.supplied_kind,
        transport: &supplied,
    };
    let deps = ChatDeps {
        config: &config,
        clients: ChatClients {
            supplied: setup.supplied.then_some(entry),
            pooled: &pooled,
        },
        state: &mut state,
        env: &env,
        stream_logger: &logger,
        counter: FixtureCounter,
        ids: &FixtureIds,
        max_event_bytes: 1_048_576,
        max_error_body_bytes: 1_048_576,
    };
    let runtime = Builder::new_current_thread().build().unwrap();
    let result = runtime.block_on(chat_call(call, deps));
    let outcome = match result {
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
                options,
            })
        }
        Err(error) => Err(error),
    };
    let mut posted_bodies = supplied.posted_bodies();
    posted_bodies.extend(pooled.posted_bodies());
    let mut posted_headers = supplied.posted_headers();
    posted_headers.extend(pooled.posted_headers());
    let details = state.model_call_details.clone();
    let transforms = std::mem::take(&mut *config.transforms.lock().unwrap());
    let calls = std::mem::take(&mut *config.calls.lock().unwrap());
    let logger_calls = std::mem::take(&mut *logger.calls.lock().unwrap());
    Run {
        outcome,
        posts_supplied: supplied.posts(),
        posts_pooled: pooled.posts(),
        posted_bodies,
        posted_headers,
        transforms,
        calls,
        logger_calls,
        details,
    }
}
