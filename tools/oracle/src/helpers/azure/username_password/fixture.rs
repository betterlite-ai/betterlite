use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_client_secret::transport::{Clock, HttpMethod, HttpReply};
use betterlite_azure::azure_username_password::transport::{HttpRequest, Transport};
use secrecy::SecretString;
use serde::Deserialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

pub(super) const NOW: i64 = 1_700_000_000;

#[derive(Clone, Deserialize)]
pub(super) struct Reply {
    #[serde(default)]
    pub(super) url_contains: Option<String>,
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) error: Option<String>,
}

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<HttpRequest>>,
    pub(super) failed: AtomicBool,
}

impl FixtureTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            requests: Mutex::new(Vec::new()),
            failed: AtomicBool::new(false),
        }
    }

    pub(super) fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn posts(&self) -> Vec<HttpRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.method == HttpMethod::Post)
            .collect()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = {
            let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
            let index = replies
                .iter()
                .position(|reply| {
                    reply
                        .url_contains
                        .as_ref()
                        .is_none_or(|needle| request.url.contains(needle))
                })
                .ok_or_else(|| io::Error::other(format!("no reply for {}", request.url)))?;
            replies
                .remove(index)
                .ok_or_else(|| io::Error::other("no reply"))?
        };
        match reply.error {
            Some(message) => {
                self.failed.store(true, Ordering::SeqCst);
                Err(io::Error::other(message))
            }
            None => Ok(HttpReply {
                status: reply.status,
                body: SecretString::from(reply.body),
            }),
        }
    }
}

pub(super) struct FixtureClock {
    pub(super) now: AtomicI64,
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub(super) struct CapturedEvent {
    pub(super) level: Level,
    pub(super) message: String,
    pub(super) fields: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
pub(super) struct Capture(Arc<Mutex<Vec<CapturedEvent>>>);

impl Capture {
    pub(super) fn events(&self) -> Vec<CapturedEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Visitor {
    message: String,
    fields: BTreeMap<String, String>,
}

impl Visitor {
    fn record(&mut self, name: &str, text: String) {
        if name == "message" {
            self.message = text;
        } else {
            self.fields.insert(name.to_owned(), text);
        }
    }
}

impl Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record(field.name(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field.name(), value.to_owned());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CapturedEvent {
                level: *event.metadata().level(),
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}
