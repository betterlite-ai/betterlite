use std::collections::VecDeque;
use std::future::pending;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::http::{HeaderMap, Method};
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use tokio::task::yield_now;

use crate::roi_github::transport::Transport;

pub(super) const RELEASE_URL: &str = "http://127.0.0.1:9/releases/latest";

pub(super) enum Answer {
    Reply { status: u16, body: Vec<u8> },
    Closed,
    Hang,
}

#[derive(Clone, Debug)]
pub(super) struct Seen {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) deadline: Duration,
}

#[derive(Clone, Default)]
pub(super) struct Script {
    replies: Arc<Mutex<VecDeque<Answer>>>,
    calls: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Script {
    pub(super) fn new(replies: Vec<Answer>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            calls: Arc::new(AtomicUsize::new(0)),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(super) fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                method: request.method,
                url: request.url.to_string(),
                deadline: request.deadline,
            });
        yield_now().await;
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match next {
            Some(Answer::Reply { status, body }) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Some(Answer::Hang) => pending().await,
            Some(Answer::Closed) | None => Err(TransportError::Closed),
        }
    }
}

pub(super) fn fake_clock(start: f64) -> (Clock, Arc<Mutex<f64>>) {
    let now = Arc::new(Mutex::new(start));
    let handle = Arc::clone(&now);
    let clock: Clock = Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
    (clock, now)
}

pub(super) fn advance(now: &Mutex<f64>, seconds: f64) {
    let mut current = now.lock().unwrap_or_else(PoisonError::into_inner);
    *current = current.mul_add(1.0_f64, seconds);
}
