#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use thiserror::Error;

use super::super::error::{CharacterCallFailure, CharacterHandlerFailure};
use super::super::handler::{CharacterAnswered, CharacterHandler, HandlerArguments, HandlerCall};
use super::super::{
    CharacterCallDeps, CharacterOutcome, CreateCharacterCall, DEFAULT_PROVIDER,
    DEFAULT_TIMEOUT_SECONDS, GetCharacterCall, avideo_create_character, avideo_get_character,
    video_create_character, video_get_character,
};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_records::CharacterObject;

pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const REQUEST_TIMEOUT_SECONDS: f64 = 1234.0;
pub(crate) const INITIAL_CALL_TYPE: &str = "fixture_call_type";
pub(crate) const CHARACTER_JSON: &str =
    r#"{"id":"char-1","object":"character","created_at":1700000000,"name":"Wren"}"#;
pub(crate) const MOCK_JSON: &str = r#"{"id":"char-9","created_at":42,"name":"Mocked"}"#;

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn character() -> CharacterObject {
    serde_json::from_str(CHARACTER_JSON).expect("the character fixture parses")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(crate) fn video() -> SuppliedMedia {
    SuppliedMedia::Bytes(b"video-bytes".to_vec())
}

pub(crate) fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([("X-Test".to_owned(), "1".to_owned())])
}

// the collaborators every fixture call runs with: its redactor and B198's REQUEST_TIMEOUT seconds
const fn deps(redactor: &Redactor) -> CharacterCallDeps<'_> {
    CharacterCallDeps {
        redactor,
        request_timeout_seconds: REQUEST_TIMEOUT_SECONDS,
    }
}

// the keyword values both calls carry: the key and the user the logging update reads
fn extra_map() -> IndexMap<String, Json> {
    IndexMap::from([
        ("api_key".to_owned(), Json::Text("sk-test".to_owned())),
        ("user".to_owned(), Json::Text("max".to_owned())),
    ])
}

// both fixture calls: the create one adds api_base to the keyword entries the get one carries
struct Calls {
    create: CreateCharacterCall,
    get: GetCharacterCall,
}

fn calls() -> Calls {
    let mut extra = extra_map();
    extra.insert(
        "api_base".to_owned(),
        Json::Text("https://api.openai.com/v1".to_owned()),
    );
    let create = CreateCharacterCall {
        name: "Wren".to_owned(),
        video: video(),
        timeout: Some(DEFAULT_TIMEOUT_SECONDS),
        custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        mock_response: None,
        litellm_call_id: Some(CALL_ID.to_owned()),
        extra,
        is_async: false,
    };
    let get = GetCharacterCall {
        character_id: "char-1".to_owned(),
        timeout: Some(DEFAULT_TIMEOUT_SECONDS),
        custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        mock_response: None,
        litellm_call_id: Some(CALL_ID.to_owned()),
        extra: extra_map(),
        is_async: false,
    };
    Calls { create, get }
}

// the call with every keyword a case cares about
pub(crate) fn create_call() -> CreateCharacterCall {
    calls().create
}

pub(crate) fn get_call() -> GetCharacterCall {
    calls().get
}

// what the fake handler answers with
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    Direct,
    Deferred,
    // the failure is raised before any future exists
    Failed {
        status_code: Option<u16>,
        message: &'static str,
    },
    // the failure is the future's own answer
    DeferredFailure {
        status_code: Option<u16>,
        message: &'static str,
    },
}

#[derive(Debug, PartialEq)]
pub(crate) struct SeenCreate {
    pub(crate) name: String,
    pub(crate) video: SuppliedMedia,
    pub(crate) provider: String,
    pub(crate) litellm_params: Params,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) timeout: Duration,
    pub(crate) is_async: bool,
    pub(crate) logged: Logged,
}

#[derive(Debug, PartialEq)]
pub(crate) struct SeenGet {
    pub(crate) character_id: String,
    pub(crate) provider: String,
    pub(crate) litellm_params: Params,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) timeout: Duration,
    pub(crate) is_async: bool,
    pub(crate) logged: Logged,
}

// the logging state as the handler found it
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Logged {
    pub(crate) model: String,
    pub(crate) call_type: String,
    pub(crate) user: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
}

fn snapshot(state: &LoggingState) -> Logged {
    Logged {
        model: state.model.clone(),
        call_type: state.call_type.clone(),
        user: state.user.clone(),
        custom_llm_provider: state.custom_llm_provider.clone(),
        optional_params: state.optional_params.clone(),
        litellm_params: state.litellm_params.clone(),
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
struct FakeFailure {
    message: String,
}

fn handler_failure(status_code: Option<u16>, message: &str) -> CharacterHandlerFailure {
    CharacterHandlerFailure {
        status_code,
        message: message.to_owned(),
        source: Box::new(FakeFailure {
            message: message.to_owned(),
        }),
    }
}

async fn deferred_answer() -> Result<CharacterObject, CharacterHandlerFailure> {
    tokio::task::yield_now().await;
    Ok(character())
}

async fn deferred_failure(
    status_code: Option<u16>,
    message: &'static str,
) -> Result<CharacterObject, CharacterHandlerFailure> {
    tokio::task::yield_now().await;
    Err(handler_failure(status_code, message))
}

fn answered(reply: Reply) -> Result<CharacterAnswered<'static>, CharacterHandlerFailure> {
    match reply {
        Reply::Direct => Ok(CharacterAnswered::Done(character())),
        Reply::Deferred => Ok(CharacterAnswered::Pending(Box::pin(deferred_answer()))),
        Reply::Failed {
            status_code,
            message,
        } => Err(handler_failure(status_code, message)),
        Reply::DeferredFailure {
            status_code,
            message,
        } => Ok(CharacterAnswered::Pending(Box::pin(deferred_failure(
            status_code,
            message,
        )))),
    }
}

// the handler both operations run in tests: it records the call and the logging state, then answers the fixture's way
pub(crate) struct FakeHandler {
    pub(crate) logging: LoggingState,
    pub(crate) reply: Reply,
    pub(crate) creates: Vec<SeenCreate>,
    pub(crate) gets: Vec<SeenGet>,
}

impl FakeHandler {
    fn new(reply: Reply) -> Self {
        Self {
            logging: logging_state(),
            reply,
            creates: Vec::new(),
            gets: Vec::new(),
        }
    }
}

impl CharacterHandler for FakeHandler {
    fn logging(&mut self) -> &mut LoggingState {
        &mut self.logging
    }

    fn handle(
        &mut self,
        call: HandlerCall,
    ) -> Result<CharacterAnswered<'_>, CharacterHandlerFailure> {
        let HandlerCall {
            arguments,
            provider,
            litellm_params,
            extra_headers,
            timeout,
            is_async,
        } = call;
        let logged = snapshot(&self.logging);
        match arguments {
            HandlerArguments::Create { name, video } => self.creates.push(SeenCreate {
                name,
                video,
                provider,
                litellm_params,
                extra_headers,
                timeout,
                is_async,
                logged,
            }),
            HandlerArguments::Get { character_id } => self.gets.push(SeenGet {
                character_id,
                provider,
                litellm_params,
                extra_headers,
                timeout,
                is_async,
                logged,
            }),
        }
        answered(self.reply)
    }
}

pub(crate) struct Fixture {
    pub(crate) handler: FakeHandler,
    redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self::answering(Reply::Direct)
    }

    pub(crate) fn answering(reply: Reply) -> Self {
        Self {
            handler: FakeHandler::new(reply),
            redactor: redactor(),
        }
    }

    pub(crate) fn state(&self) -> &LoggingState {
        &self.handler.logging
    }

    pub(crate) fn create(
        &mut self,
        call: &CreateCharacterCall,
    ) -> Result<CharacterOutcome<'_>, CharacterCallFailure> {
        let handler = &mut self.handler;
        video_create_character(handler, deps(&self.redactor), call)
    }

    pub(crate) async fn acreate(
        &mut self,
        call: &CreateCharacterCall,
    ) -> Result<CharacterObject, CharacterCallFailure> {
        let handler = &mut self.handler;
        avideo_create_character(handler, deps(&self.redactor), call).await
    }

    pub(crate) fn get(
        &mut self,
        call: &GetCharacterCall,
    ) -> Result<CharacterOutcome<'_>, CharacterCallFailure> {
        let handler = &mut self.handler;
        video_get_character(handler, deps(&self.redactor), call)
    }

    pub(crate) async fn aget(
        &mut self,
        call: &GetCharacterCall,
    ) -> Result<CharacterObject, CharacterCallFailure> {
        let handler = &mut self.handler;
        avideo_get_character(handler, deps(&self.redactor), call).await
    }
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

fn logging_state() -> LoggingState {
    LoggingState::new(
        LoggingInit {
            model: "character".to_owned(),
            call_type: INITIAL_CALL_TYPE.to_owned(),
            start_time: moment(),
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}
