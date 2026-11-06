mod batch;
mod buffer;
mod failure;
mod success;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::galileo::admission::{CallbackKwargs, StandardLoggingObject, SuccessEvent};
use crate::galileo::delivery::config::GalileoConfig;
use crate::galileo::delivery::headers::Header;
use crate::galileo::delivery::transport::{HttpResponse, TraceIds, Transport, TransportError};
use crate::galileo::delivery::{GalileoDelivery, State};
use crate::galileo_payloads::content::{GalileoKwargs, ResponseObj};
use crate::galileo_payloads::timestamps::RawTimestamp;

pub(super) const API_KEY_ENV: &[(&str, &str)] = &[
    ("GALILEO_API_KEY", "key-0123456789abcdef"),
    ("GALILEO_PROJECT_ID", "project-1"),
];

pub(super) const CHAT_OUTPUT: &str = r#"{"content": "Hi", "role": "assistant", "tool_calls": null, "function_call": null, "provider_specific_fields": null}"#;

pub(super) fn delivery(environment: &[(&str, &str)]) -> GalileoDelivery {
    let variables: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    GalileoDelivery {
        config: GalileoConfig::from_environment(&variables),
        state: Mutex::new(State::default()),
    }
}

pub(super) fn delivery_with_batch(
    environment: &[(&str, &str)],
    batch_size: usize,
) -> GalileoDelivery {
    let mut delivery = delivery(environment);
    delivery.config.batch_size = batch_size;
    delivery
}

pub(super) fn standard_object() -> StandardLoggingObject {
    StandardLoggingObject {
        call_type: Some("acompletion".to_owned()),
        model: Some("gpt-5.2".to_owned()),
        prompt_tokens: Some(3),
        completion_tokens: Some(4),
        total_tokens: Some(7),
        response_cost: Some(Decimal::new(1, 3)),
        start_time: Some(Decimal::from(1_779_710_400_i64)),
        end_time: Some(Decimal::new(1_779_710_401_500, 3)),
    }
}

pub(super) fn kwargs(slo: Option<StandardLoggingObject>, messages: Option<Json>) -> CallbackKwargs {
    CallbackKwargs {
        standard_logging_object: slo,
        model: Some("gpt-4o".to_owned()),
        content: GalileoKwargs {
            call_type: Some("acompletion".to_owned()),
            messages,
            functions: None,
            tools: None,
            input: None,
        },
    }
}

pub(super) fn event<'event>(
    kwargs: &'event CallbackKwargs,
    response: Option<&'event ResponseObj>,
) -> SuccessEvent<'event> {
    SuccessEvent {
        kwargs,
        response,
        start_time: None,
        end_time: None,
        local_offset: FixedOffset::east_opt(0).expect("UTC is a valid offset"),
    }
}

pub(super) fn chat_response(content: &str) -> ResponseObj {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("assistant".to_owned()));
    message.insert("content".to_owned(), Json::Text(content.to_owned()));
    ResponseObj::Chat {
        message: Some(Json::Object(message)),
    }
}

pub(super) fn user_messages(content: &str) -> Json {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert("content".to_owned(), Json::Text(content.to_owned()));
    Json::List(vec![Json::Object(message)])
}

pub(super) fn naive(value: &str) -> RawTimestamp {
    RawTimestamp::Naive(
        NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f").expect("a naive datetime"),
    )
}

pub(super) fn aware(value: &str) -> RawTimestamp {
    RawTimestamp::Aware(DateTime::parse_from_rfc3339(value).expect("an aware datetime"))
}

pub(super) fn offset_minutes(minutes: i32) -> FixedOffset {
    let seconds = minutes.checked_mul(60).expect("minutes fit in seconds");
    FixedOffset::east_opt(seconds).expect("a valid fixed offset")
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(super) fn decimal(value: &str) -> Json {
    Json::Number(serde_json::Number::from_str(value).expect("a valid number literal"))
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn records(delivery: &GalileoDelivery) -> Vec<IndexMap<String, Json>> {
    delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .records
        .clone()
}

pub(super) fn prefill(delivery: &GalileoDelivery, count: usize) {
    let mut state = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for index in 0..count {
        let mut record = IndexMap::new();
        record.insert("input_text".to_owned(), Json::Text(index.to_string()));
        record.insert("output_text".to_owned(), text("out"));
        state.records.push(record);
    }
}

#[derive(Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<HttpResponse>>,
    calls: Mutex<Vec<IndexMap<String, Json>>>,
}

impl ScriptedTransport {
    pub(super) fn replying(status: u16) -> Self {
        let reply = HttpResponse {
            status,
            body: String::new(),
        };
        Self {
            replies: Mutex::new(VecDeque::from([reply])),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<IndexMap<String, Json>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self, url: &str) -> Reply {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| TransportError {
                url: url.to_owned(),
                reason: "no scripted reply".to_owned(),
            })
    }
}

type Reply = Result<HttpResponse, TransportError>;

impl Transport for ScriptedTransport {
    fn get(&self, url: &str, _headers: &[Header]) -> Reply {
        self.answer(url)
    }

    fn post_form(&self, url: &str, _headers: &[Header], _form: &[(&str, &str)]) -> Reply {
        self.answer(url)
    }

    fn post_json(&self, url: &str, _headers: &[Header], payload: &IndexMap<String, Json>) -> Reply {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        self.answer(url)
    }
}

#[derive(Default)]
pub(super) struct CountingIds(AtomicUsize);

impl TraceIds for CountingIds {
    fn next_id(&self) -> String {
        format!("id-{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<(Level, String)>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl CapturedLogs {
    pub(super) fn matching(&self, level: Level, needle: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(event_level, message)| *event_level == level && message.contains(needle))
            .map(|(_, message)| message.clone())
            .collect()
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), message.0));
    }
}
