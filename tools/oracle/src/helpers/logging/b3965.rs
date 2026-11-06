use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex, PoisonError};

use betterlite_logging::galileo::admission::{
    CallbackKwargs, SuccessEvent, async_log_failure_event, async_log_success_event,
};
use betterlite_logging::galileo::delivery::config::GalileoConfig;
use betterlite_logging::galileo::delivery::transport::HttpResponse;
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
use transport::{CountedIds, Scripted};

mod logs;
mod transport;

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
