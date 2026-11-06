use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_logging::galileo::admission::{
    CallbackKwargs, SuccessEvent, async_log_failure_event, async_log_success_event,
};
use betterlite_logging::galileo::delivery::config::GalileoConfig;
use betterlite_logging::galileo::delivery::headers::Header;
use betterlite_logging::galileo::delivery::transport::{
    HttpResponse, TraceIds, Transport, TransportError,
};
use betterlite_logging::galileo::delivery::{GalileoDelivery, State};
use betterlite_logging::galileo_payloads::content::ResponseObj;
use betterlite_logging::galileo_payloads::timestamps::RawTimestamp;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};
use logs::Logs;

mod logs;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    batch_size: Option<usize>,
    #[serde(default)]
    offset_minutes: Option<i32>,
    #[serde(default)]
    replies: Vec<Reply>,
    actions: Vec<Action>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    #[serde(rename = "do")]
    step: Step,
    #[serde(default)]
    kwargs: Box<CallbackKwargs>,
    #[serde(default)]
    response: Option<ResponseObj>,
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    end_time: Option<String>,
    #[serde(default)]
    count: usize,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Success,
    Failure,
    Prefill,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let delivery = build_delivery(input);
        let transport = build_transport(input);
        let ids = CountedIds(AtomicUsize::new(1));
        let local_offset = offset(input.offset_minutes.unwrap_or(0_i32))?;
        let logs = Logs::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        with_default(subscriber, || {
            perform(&input.actions, &delivery, &transport, &ids, local_offset)
        })?;
        let records = delivery
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .records
            .clone();
        Ok(json!({
            "records": records,
            "calls": transport.calls(),
            "warnings": logs.messages(),
        }))
    }
}

fn build_delivery(input: &Input) -> GalileoDelivery {
    let mut config = GalileoConfig::from_environment(&input.environment);
    if let Some(batch_size) = input.batch_size {
        config.batch_size = batch_size;
    }
    GalileoDelivery {
        config,
        state: Mutex::new(State::default()),
    }
}

fn build_transport(input: &Input) -> Scripted {
    let replies: VecDeque<HttpResponse> = input
        .replies
        .iter()
        .map(|reply| HttpResponse {
            status: reply.status,
            body: reply.body.clone(),
        })
        .collect();
    Scripted {
        replies: Mutex::new(replies),
        calls: Mutex::new(Vec::new()),
    }
}

fn perform(
    actions: &[Action],
    delivery: &GalileoDelivery,
    transport: &Scripted,
    ids: &CountedIds,
    local_offset: FixedOffset,
) -> HelperResult {
    for action in actions {
        match action.step {
            Step::Prefill => prefill(delivery, action.count),
            Step::Failure => async_log_failure_event(action.kwargs.as_ref()),
            Step::Success => {
                let event = SuccessEvent {
                    kwargs: action.kwargs.as_ref(),
                    response: action.response.as_ref(),
                    start_time: action
                        .start_time
                        .as_deref()
                        .map(raw_timestamp)
                        .transpose()?,
                    end_time: action.end_time.as_deref().map(raw_timestamp).transpose()?,
                    local_offset,
                };
                async_log_success_event(delivery, transport, ids, &event);
            }
        }
    }
    Ok(Value::Null)
}

fn offset(minutes: i32) -> Result<FixedOffset, Box<HelperError>> {
    let seconds = minutes
        .checked_mul(60)
        .ok_or_else(|| failure("offset_minutes is out of range"))?;
    FixedOffset::east_opt(seconds).ok_or_else(|| failure("offset_minutes is out of range"))
}

fn raw_timestamp(value: &str) -> Result<RawTimestamp, Box<HelperError>> {
    if let Ok(aware_time) = DateTime::parse_from_rfc3339(value) {
        return Ok(RawTimestamp::Aware(aware_time));
    }
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .map(RawTimestamp::Naive)
        .map_err(|error| failure(&format!("invalid datetime {value}: {error}")))
}

fn prefill(delivery: &GalileoDelivery, count: usize) {
    let mut state = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for index in 0..count {
        let mut record = IndexMap::new();
        record.insert("input_text".to_owned(), Json::Text(index.to_string()));
        record.insert("output_text".to_owned(), Json::Text("out".to_owned()));
        state.records.push(record);
    }
}

fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "HelperFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

struct Scripted {
    replies: Mutex<VecDeque<HttpResponse>>,
    calls: Mutex<Vec<Value>>,
}

struct Call<'call> {
    method: &'call str,
    url: &'call str,
    headers: &'call [Header],
    form: Option<Value>,
    payload: Option<Value>,
}

impl Scripted {
    fn answer(&self, url: &str, call: Value) -> Result<HttpResponse, TransportError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| TransportError {
                url: url.to_owned(),
                reason: "no scripted reply".to_owned(),
            })
    }

    fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn recorded(call: &Call<'_>) -> Value {
    let names: BTreeSet<String> = call
        .headers
        .iter()
        .map(|header| header.name.clone())
        .collect();
    json!({
        "method": call.method,
        "url": call.url,
        "headers": names.into_iter().collect::<Vec<String>>(),
        "form": call.form.clone(),
        "payload": call.payload.clone(),
    })
}

impl Transport for Scripted {
    fn get(&self, url: &str, headers: &[Header]) -> Result<HttpResponse, TransportError> {
        let call = recorded(&Call {
            method: "GET",
            url,
            headers,
            form: None,
            payload: None,
        });
        self.answer(url, call)
    }

    fn post_form(
        &self,
        url: &str,
        headers: &[Header],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError> {
        let fields: BTreeMap<String, String> = form
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let call = recorded(&Call {
            method: "POST",
            url,
            headers,
            form: Some(json!(fields)),
            payload: None,
        });
        self.answer(url, call)
    }

    fn post_json(
        &self,
        url: &str,
        headers: &[Header],
        payload: &IndexMap<String, Json>,
    ) -> Result<HttpResponse, TransportError> {
        let call = recorded(&Call {
            method: "POST",
            url,
            headers,
            form: None,
            payload: Some(payload_view(payload)),
        });
        self.answer(url, call)
    }
}

fn payload_view(payload: &IndexMap<String, Json>) -> Value {
    let mut whole = json!(payload);
    let traces = whole
        .as_object_mut()
        .and_then(|fields| fields.remove("traces"))
        .unwrap_or(Value::Null);
    let trace_ids: Vec<Value> = traces
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|trace| json!([trace.pointer("/id"), trace.pointer("/spans/0/id")]))
                .collect()
        })
        .unwrap_or_default();
    json!({"trace_ids": trace_ids, "rest": whole})
}

struct CountedIds(AtomicUsize);

impl TraceIds for CountedIds {
    fn next_id(&self) -> String {
        let number = self.0.fetch_add(1, Ordering::SeqCst);
        format!("00000000-0000-0000-0000-{number:012x}")
    }
}
