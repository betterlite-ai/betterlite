mod auth;
mod buffer;
mod config;
mod diagnostics;
mod flush;
mod headers;
mod health;

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tracing::subscriber::{DefaultGuard, set_default};

use super::super::buffer::append;
use super::super::config::GalileoConfig;
use super::super::headers::Header;
use super::super::transport::{HttpResponse, TraceIds, Transport, TransportError};
use super::super::{GalileoDelivery, Record, State};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Call {
    pub(super) method: &'static str,
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) form: Vec<(String, String)>,
    pub(super) payload: Option<IndexMap<String, Json>>,
}

pub(super) type Reply = Result<HttpResponse, TransportError>;

#[derive(Default)]
pub(super) struct ScriptedTransport {
    pub(super) replies: Mutex<VecDeque<Reply>>,
    pub(super) calls: Mutex<Vec<Call>>,
    pub(super) append_during_post: Option<(Arc<GalileoDelivery>, Record)>,
}

impl ScriptedTransport {
    pub(super) fn replying(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
            append_during_post: None,
        }
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn answer(&self, call: Call) -> Reply {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        next.unwrap_or_else(|| {
            Err(TransportError {
                url: String::new(),
                reason: "no scripted reply".to_owned(),
            })
        })
    }
}

fn call_to(method: &'static str, url: &str, headers: &[Header]) -> Call {
    Call {
        method,
        url: url.to_owned(),
        headers: headers
            .iter()
            .map(|header| (header.name.clone(), header.value.expose_secret().to_owned()))
            .collect(),
        form: Vec::new(),
        payload: None,
    }
}

impl Transport for ScriptedTransport {
    fn get(&self, url: &str, headers: &[Header]) -> Reply {
        self.answer(call_to("GET", url, headers))
    }

    fn post_form(&self, url: &str, headers: &[Header], form: &[(&str, &str)]) -> Reply {
        let mut call = call_to("POST", url, headers);
        call.form = form
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        self.answer(call)
    }

    fn post_json(&self, url: &str, headers: &[Header], payload: &IndexMap<String, Json>) -> Reply {
        if let Some((delivery, record)) = &self.append_during_post {
            append(delivery, record.clone());
        }
        let mut call = call_to("POST", url, headers);
        call.payload = Some(payload.clone());
        self.answer(call)
    }
}

#[derive(Default)]
pub(super) struct CountingIds(AtomicUsize);

impl TraceIds for CountingIds {
    fn next_id(&self) -> String {
        format!("id-{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}

pub(super) fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        body: body.to_owned(),
    }
}

pub(super) fn unreachable_host() -> Reply {
    Err(TransportError {
        url: "https://galileo.example".to_owned(),
        reason: "connection refused".to_owned(),
    })
}

pub(super) fn delivery(variables: &[(&str, &str)]) -> GalileoDelivery {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    GalileoDelivery {
        config: GalileoConfig::from_environment(&environment),
        state: Mutex::new(State::default()),
    }
}

pub(super) const API_KEY_ENV: &[(&str, &str)] = &[
    ("GALILEO_API_KEY", "key-0123456789abcdef"),
    ("GALILEO_PROJECT_ID", "project-1"),
];

pub(super) const TOKEN_ENV: &[(&str, &str)] = &[
    ("GALILEO_USERNAME", "user"),
    ("GALILEO_PASSWORD", "pass"),
    ("GALILEO_PROJECT_ID", "project-1"),
    ("GALILEO_BASE_URL", "https://galileo.example/"),
];

pub(super) fn record(label: &str) -> Record {
    let mut record = Record::new();
    record.insert("input_text".to_owned(), Json::Text(label.to_owned()));
    record.insert("output_text".to_owned(), Json::Text("out".to_owned()));
    record
}

pub(super) fn fill(delivery: &GalileoDelivery, count: usize) {
    for index in 0..count {
        append(delivery, record(&index.to_string()));
    }
}

pub(super) fn labels(delivery: &GalileoDelivery) -> Vec<String> {
    let state = delivery.state.lock().unwrap();
    state
        .records
        .iter()
        .filter_map(|record| match record.get("input_text") {
            Some(Json::Text(label)) => Some(label.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn cached_header_names(delivery: &GalileoDelivery) -> Option<Vec<String>> {
    let state = delivery.state.lock().unwrap();
    state
        .headers
        .as_ref()
        .map(|headers| headers.iter().map(|header| header.name.clone()).collect())
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("lock"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
