use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::resource_config_selection::responses::{
    ResponsesConfigTag, ResponsesModelFacts,
};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_resources::request::ListInputItemsQuery;
use betterlite_openai::response_resources::response::{TypedReply, response_reply};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::api::responses_lifecycle_call::error::LifecycleFailure;
use crate::api::responses_lifecycle_call::executor::{ReadRequest, ResponsesReadExecutor};
use crate::api::responses_lifecycle_call::input::{ResponsesInputItemsCall, ResponsesRetrieveCall};
use crate::api::responses_lifecycle_call::service::{LifecycleLog, ResponsesLifecycle};
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{ClientKind, ReadOutcome};
use crate::api::vector_store_http_write::error::BaseLlmException;

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_mins(100);

// the logging state a request starts from, before the lifecycle call writes its own fields
const INIT: &str = r#"{"model":"responses","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) fn map(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the map fixture is valid")
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn retrieve(text: &str) -> ResponsesRetrieveCall {
    serde_json::from_str(text).expect("the retrieve fixture parses")
}

pub(crate) fn input_items(text: &str) -> ResponsesInputItemsCall {
    serde_json::from_str(text).expect("the input-items fixture parses")
}

#[derive(Clone)]
pub(crate) enum Answer {
    Retrieved {
        body: &'static str,
        header: Option<(&'static str, &'static str)>,
    },
    Items(&'static str),
    Provider {
        status: u16,
        message: &'static str,
    },
}

#[derive(Clone)]
pub(crate) struct Recorded {
    pub(crate) tag: ResponsesConfigTag,
    pub(crate) response_id: String,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    pub(crate) query: Option<ListInputItemsQuery>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) mode: ClientKind,
    pub(crate) logged_optional: IndexMap<String, Json>,
    pub(crate) logged_litellm: IndexMap<String, Json>,
}

// the B2416 read seam with no socket: each read pops a queued answer and records what it was handed
pub(crate) struct FakeExecutor {
    answers: Mutex<VecDeque<Answer>>,
    records: Mutex<Vec<Recorded>>,
}

impl FakeExecutor {
    fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            records: Mutex::default(),
        }
    }

    pub(crate) fn records(&self) -> Vec<Recorded> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponsesReadExecutor for FakeExecutor {
    fn read(
        &self,
        request: ReadRequest<'_, '_, '_>,
    ) -> impl Future<Output = Result<ReadOutcome, ReadError>> + Send {
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test queued an answer for every read");
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                tag: request.tag.clone(),
                response_id: request.response_id.to_owned(),
                api_base: request.params.api_base.clone(),
                api_key: request
                    .params
                    .api_key
                    .as_ref()
                    .map(|key| key.expose_secret().to_owned()),
                extra_headers: request.extra_headers.cloned(),
                extra_body: request.extra_body.cloned(),
                query: request.query.cloned(),
                timeout: request.timeout,
                mode: request.mode,
                logged_optional: request.log.state.optional_params.clone(),
                logged_litellm: request.log.state.litellm_params.clone(),
            });
        std::future::ready(reply(&answer))
    }
}

fn reply(answer: &Answer) -> Result<ReadOutcome, ReadError> {
    match answer {
        Answer::Retrieved { body, header } => {
            let mut headers = IndexMap::new();
            if let Some((name, value)) = header {
                headers.insert((*name).to_owned(), (*value).to_owned());
            }
            let raw = RawResponse {
                status: 200,
                headers: &headers,
                text: *body,
            };
            let typed = response_reply(&raw).map_err(ReadError::Response)?;
            Ok(ReadOutcome::Response(Box::new(typed)))
        }
        Answer::Items(body) => Ok(ReadOutcome::InputItems(json(*body))),
        Answer::Provider { status, message } => Err(ReadError::Provider(BaseLlmException {
            status_code: *status,
            message: (*message).to_owned(),
            headers: BTreeMap::new(),
            status_code_is_synthesized: false,
        })),
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn pre_post<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

pub(crate) struct Fixture {
    pub(crate) service: ResponsesLifecycle<FakeExecutor>,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

// the log one fixture read hands the service, built from the fixture's own sink, state and clock
fn log_of<'env>(
    service: &'env ResponsesLifecycle<FakeExecutor>,
    sink: &'env Sink,
    state: &'env mut LoggingState,
    now: DateTime<FixedOffset>,
) -> LifecycleLog<'env, 'env> {
    LifecycleLog {
        state,
        env: pre_post(&service.redactor, sink, now),
    }
}

impl Fixture {
    pub(crate) fn new(answers: Vec<Answer>) -> Self {
        Self::with_timeout(answers, DEFAULT_TIMEOUT)
    }

    pub(crate) fn with_timeout(answers: Vec<Answer>, default_timeout: Duration) -> Self {
        let init: LoggingInit = serde_json::from_str(INIT).expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            service: ResponsesLifecycle {
                executor: FakeExecutor::new(answers),
                default_timeout,
                facts: ResponsesModelFacts::default(),
                redactor,
            },
            sink: Sink,
            state,
            now,
        }
    }

    pub(crate) fn state(&self) -> &LoggingState {
        &self.state
    }

    pub(crate) async fn retrieve(
        &mut self,
        call: &ResponsesRetrieveCall,
    ) -> Result<TypedReply, Box<LifecycleFailure>> {
        let log = log_of(&self.service, &self.sink, &mut self.state, self.now);
        self.service.get_responses(call, log).await
    }

    pub(crate) async fn aretrieve(
        &mut self,
        call: &ResponsesRetrieveCall,
    ) -> Result<TypedReply, Box<LifecycleFailure>> {
        let log = log_of(&self.service, &self.sink, &mut self.state, self.now);
        self.service.aget_responses(call, log).await
    }

    pub(crate) async fn list(
        &mut self,
        call: &ResponsesInputItemsCall,
    ) -> Result<Json, Box<LifecycleFailure>> {
        let log = log_of(&self.service, &self.sink, &mut self.state, self.now);
        self.service.list_input_items(call, log).await
    }

    pub(crate) async fn alist(
        &mut self,
        call: &ResponsesInputItemsCall,
    ) -> Result<Json, Box<LifecycleFailure>> {
        let log = log_of(&self.service, &self.sink, &mut self.state, self.now);
        self.service.alist_input_items(call.clone(), log).await
    }
}
