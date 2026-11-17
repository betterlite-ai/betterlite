#![cfg(test)]

use std::convert::Infallible;
use std::mem::take;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::converse_response::request::{OptionalParams, RawReply, ReplyRequest};
use crate::converse_response::transform::{TransformCall, TransformError, transform_response};
use crate::converse_response::wire::{Message, ModelResponse};
use crate::converse_usage::transform::ReasoningTokenCounter;
use crate::image_call::plan::LogContext;

pub(super) const MODEL: &str = "fixture-model";
pub(super) const FIRST_ID: &str = "11111111-2222-3333-4444-555555555555";
pub(super) const SECOND_ID: &str = "22222222-3333-4444-5555-666666666666";
const INIT: &str = r#"{"model":"fixture-model","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

// Every conversion reads its clock and its ids from the fixture, so no test touches the wall clock.
pub(super) struct Fixture {
    pub(super) model: String,
    pub(super) messages: Json,
    pub(super) data: IndexMap<String, Json>,
    pub(super) json_mode: Option<bool>,
    pub(super) optional: OptionalParams,
    pub(super) mappings: IndexMap<String, String>,
    pub(super) supplied: ModelResponse,
    pub(super) headers: Vec<(String, String)>,
    pub(super) ids: Vec<Uuid>,
    pub(super) now: SystemTime,
    pub(super) counter: i64,
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        model: MODEL.to_owned(),
        messages: json(r#"[{"role": "user", "content": "hi"}]"#),
        data: object(r#"{"model": "fixture-model"}"#),
        json_mode: None,
        optional: OptionalParams::default(),
        mappings: IndexMap::new(),
        supplied: supplied("chatcmpl-supplied", None),
        headers: vec![(String::from("x-amzn-requestid"), String::from("req-1"))],
        ids: Vec::new(),
        now: instant(1_700_000_000),
        counter: 2,
    }
}

pub(super) struct Run {
    pub(super) response: Result<ModelResponse, TransformError<Infallible>>,
    pub(super) details: IndexMap<String, Json>,
}

impl Run {
    pub(super) fn response(&self) -> &ModelResponse {
        self.response.as_ref().unwrap()
    }

    pub(super) fn message(&self) -> &Message {
        &self.response().choices[0].message
    }

    pub(super) fn finish(&self) -> &str {
        &self.response().choices[0].finish_reason
    }

    pub(super) fn failure(&self) -> &TransformError<Infallible> {
        self.response.as_ref().unwrap_err()
    }

    pub(super) fn status(&self) -> Option<u16> {
        self.response
            .as_ref()
            .err()
            .and_then(TransformError::status)
    }

    pub(super) fn detail(&self, name: &str) -> &Json {
        self.details.get(name).unwrap()
    }
}

struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

struct FixedIds {
    values: Mutex<Vec<Uuid>>,
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        if values.is_empty() {
            return Uuid::nil();
        }
        values.remove(0)
    }
}

struct FixedCounter {
    count: i64,
}

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(self.count)
    }
}

pub(super) fn convert(fixture: &Fixture, body: &str) -> Run {
    convert_bytes(fixture, body.as_bytes())
}

fn logging_fixture<'borrow>(
    redactor: &'borrow Redactor,
    sink: &'borrow DiscardingSink,
) -> (LoggingState, PrePostEnv<'borrow>) {
    let init: LoggingInit = serde_json::from_str(INIT).expect("the logging fixture is valid");
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging fixture builds");
    let env = PrePostEnv {
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339(NOW).expect("the fixture clock parses"),
    };
    (state, env)
}

pub(super) fn convert_bytes(fixture: &Fixture, body: &[u8]) -> Run {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
    let sink = DiscardingSink;
    let (mut state, env) = logging_fixture(&redactor, &sink);
    let ids = FixedIds {
        values: Mutex::new(fixture.ids.clone()),
    };
    let counter = FixedCounter {
        count: fixture.counter,
    };
    let request = ReplyRequest {
        model: &fixture.model,
        messages: &fixture.messages,
        data: RawBody::Object(fixture.data.clone()),
        json_mode: fixture.json_mode,
        optional: &fixture.optional,
        tool_name_mappings: &fixture.mappings,
    };
    let reply = RawReply {
        headers: &fixture.headers,
        body,
    };
    let response = transform_response(
        TransformCall {
            request,
            log: LogContext {
                state: &mut state,
                env: &env,
            },
            now: fixture.now,
        },
        &reply,
        fixture.supplied.clone(),
        &ids,
        &counter,
    );
    Run {
        response,
        details: take(&mut state.model_call_details),
    }
}

pub(super) fn optional_params(source: &str) -> OptionalParams {
    serde_json::from_str(source).expect("the optional params parse")
}

pub(super) fn supplied(id: &str, fingerprint: Option<&str>) -> ModelResponse {
    ModelResponse {
        id: id.to_owned(),
        created: 0,
        model: String::from("previous-model"),
        object: String::from("chat.completion"),
        system_fingerprint: fingerprint.map(str::to_owned),
        choices: Vec::new(),
        usage: None,
        service_tier: None,
        trace: None,
    }
}

pub(super) fn with_tier(mut value: ModelResponse, tier: &str, trace: &str) -> ModelResponse {
    value.service_tier = Some(json(tier));
    value.trace = Some(json(trace));
    value
}

pub(super) fn instant(seconds: i64) -> SystemTime {
    let magnitude = Duration::from_secs(seconds.unsigned_abs());
    if seconds < 0 {
        return UNIX_EPOCH
            .checked_sub(magnitude)
            .expect("the fixture clock is representable");
    }
    UNIX_EPOCH
        .checked_add(magnitude)
        .expect("the fixture clock is representable")
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture JSON parses")
}

pub(super) fn object(source: &str) -> IndexMap<String, Json> {
    match json(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is not an object")
        }
    }
}

pub(super) fn rendered<Payload>(value: &Payload) -> Json
where
    Payload: serde::Serialize,
{
    json(&serde_json::to_string(value).expect("the value serializes"))
}
