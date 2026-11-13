use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_logging::galileo::delivery::auth::ensure_headers;
use betterlite_logging::galileo::delivery::buffer::append;
use betterlite_logging::galileo::delivery::config::GalileoConfig;
use betterlite_logging::galileo::delivery::flush::{deliver, ingest_request};
use betterlite_logging::galileo::delivery::headers::{Header, redact_headers};
use betterlite_logging::galileo::delivery::health::check;
use betterlite_logging::galileo::delivery::transport::{
    HttpResponse, TraceIds, Transport, TransportError,
};
use betterlite_logging::galileo::delivery::{GalileoDelivery, Record, State};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    #[serde(default)]
    replies: Vec<Reply>,
    actions: Vec<Action>,
}

#[derive(Deserialize)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Action {
    Push { label: String },
    Flush,
    HealthCheck,
    EnsureHeaders,
    IngestUrl,
    Redact { headers: IndexMap<String, String> },
}

struct Scripted {
    replies: Mutex<VecDeque<HttpResponse>>,
    calls: Mutex<Vec<Value>>,
}

impl Scripted {
    fn answer(&self, call: Value, url: &str) -> Result<HttpResponse, TransportError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        next.ok_or_else(|| TransportError {
            url: url.to_owned(),
            reason: "no scripted reply".to_owned(),
        })
    }
}

impl Transport for Scripted {
    fn get(&self, url: &str, headers: &[Header]) -> Result<HttpResponse, TransportError> {
        let call = json!({"method": "GET", "url": url, "headers": exposed(headers)});
        self.answer(call, url)
    }

    fn post_form(
        &self,
        url: &str,
        headers: &[Header],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError> {
        let fields: BTreeMap<&str, &str> = form.iter().copied().collect();
        let call = json!({
            "method": "POST", "url": url, "headers": exposed(headers), "form": fields,
        });
        self.answer(call, url)
    }

    fn post_json(
        &self,
        url: &str,
        headers: &[Header],
        payload: &IndexMap<String, Json>,
    ) -> Result<HttpResponse, TransportError> {
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
        let call = json!({
            "method": "POST",
            "url": url,
            "headers": exposed(headers),
            "form": null,
            "payload": {"trace_ids": trace_ids, "rest": whole},
        });
        self.answer(call, url)
    }
}

struct CountedIds(AtomicUsize);

impl TraceIds for CountedIds {
    fn next_id(&self) -> String {
        let number = self.0.fetch_add(1, Ordering::SeqCst);
        format!("00000000-0000-0000-0000-{number:012x}")
    }
}

fn exposed(headers: &[Header]) -> Value {
    json!(
        headers
            .iter()
            .map(|header| (header.name.as_str(), header.value.expose_secret()))
            .collect::<BTreeMap<&str, &str>>()
    )
}

fn labels(delivery: &GalileoDelivery) -> Vec<Value> {
    let state = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    state
        .records
        .iter()
        .filter_map(|record| record.get("input_text"))
        .map(|label| json!(label))
        .collect()
}

fn record(label: &str) -> Record {
    let mut fields = Record::new();
    fields.insert("input_text".to_owned(), Json::Text(label.to_owned()));
    fields.insert("output_text".to_owned(), Json::Text("out".to_owned()));
    fields
}

fn perform(
    action: &Action,
    delivery: &GalileoDelivery,
    transport: &Scripted,
    ids: &CountedIds,
) -> Value {
    match action {
        Action::Push { label } => {
            append(delivery, record(label));
            Value::Null
        }
        Action::Flush => {
            deliver(delivery, transport, ids);
            Value::Null
        }
        Action::HealthCheck => json!(check(delivery, transport)),
        Action::EnsureHeaders => json!(ensure_headers(delivery, transport)),
        Action::IngestUrl => {
            let records = delivery
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .records
                .clone();
            json!(ingest_request(delivery, &records, ids).map(|(url, _payload)| url))
        }
        Action::Redact { headers } => {
            let pairs: Vec<Header> = headers
                .iter()
                .map(|(name, value)| Header {
                    name: name.clone(),
                    value: SecretString::from(value.as_str()),
                })
                .collect();
            json!(
                redact_headers(&pairs)
                    .into_iter()
                    .collect::<BTreeMap<_, _>>()
            )
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let delivery = GalileoDelivery {
            config: GalileoConfig::from_environment(&input.environment),
            state: Mutex::new(State::default()),
        };
        let transport = Scripted {
            replies: Mutex::new(
                input
                    .replies
                    .iter()
                    .map(|reply| HttpResponse {
                        status: reply.status,
                        body: reply.body.clone(),
                    })
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
        };
        let ids = CountedIds(AtomicUsize::new(1));
        let results: Vec<Value> = input
            .actions
            .iter()
            .map(|action| perform(action, &delivery, &transport, &ids))
            .collect();
        let stored = delivery
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .headers
            .clone();
        Ok(json!({
            "results": results,
            "calls": transport.calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            "records": labels(&delivery),
            "headers": stored.map(|headers| {
                redact_headers(&headers).into_iter().collect::<BTreeMap<_, _>>()
            }),
        }))
    }
}
