use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;

use super::Clock;
use super::warnings::{FeedWarning, WarningSink};
use crate::roi_github::transport::Transport;

#[derive(Clone, Debug)]
pub enum Answer {
    Reply { status: u16, body: Vec<u8> },
    Closed,
}

#[must_use]
pub fn reply(status: u16, text: &str) -> Answer {
    Answer::Reply {
        status,
        body: text.as_bytes().to_vec(),
    }
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub url: String,
    pub timeout_seconds: u64,
}

#[derive(Clone)]
pub struct Script {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Script {
    #[must_use]
    pub fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let seen = Seen {
            url: request.url.to_string(),
            timeout_seconds: request.deadline.as_secs(),
        };
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(match answer {
            Some(Answer::Reply { status, body }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Some(Answer::Closed) | None => Err(TransportError::Closed),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FixedClock {
    pub now: f64,
}

impl Clock for FixedClock {
    fn now_seconds(&self) -> f64 {
        self.now
    }
}

pub struct StepClock {
    pub values: Mutex<VecDeque<f64>>,
    pub last: f64,
    pub reads: AtomicUsize,
}

impl StepClock {
    #[must_use]
    pub fn new<Values>(values: Values) -> Self
    where
        Values: IntoIterator<Item = f64>,
    {
        let queue: VecDeque<f64> = values.into_iter().collect();
        let last = queue.back().copied().unwrap_or(0.0_f64);
        Self {
            values: Mutex::new(queue),
            last,
            reads: AtomicUsize::new(0),
        }
    }
}

impl Clock for StepClock {
    fn now_seconds(&self) -> f64 {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or(self.last)
    }
}

#[derive(Clone, Default)]
pub struct RecordingWarnings {
    entries: Arc<Mutex<Vec<FeedWarning>>>,
}

impl RecordingWarnings {
    #[must_use]
    pub fn warnings(&self) -> Vec<FeedWarning> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl WarningSink for RecordingWarnings {
    fn warn(&self, warning: &FeedWarning) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(warning.clone());
    }
}
