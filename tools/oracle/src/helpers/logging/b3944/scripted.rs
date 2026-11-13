use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::generic_api_logger::generic_logger::retry::RetrySleeper;
use betterlite_logging::newrelic_metrics::clock::Clock;
use betterlite_logging::newrelic_metrics::logger::NewRelicMetricsLogger;
use betterlite_logging::newrelic_metrics::queue::push_record;
use betterlite_logging::newrelic_metrics::records::NewRelicMetricRecord;
use betterlite_logging::newrelic_metrics::transport::{
    NewRelicPost, NewRelicSendError, NewRelicTransport,
};
use http::HeaderMap;

use crate::helpers::HelperError;

use super::ReplyInput;
use super::failure;

pub(super) struct ScriptedClock {
    values: Arc<Mutex<VecDeque<Duration>>>,
    fallback: Duration,
}

impl ScriptedClock {
    pub(super) fn new(seconds: &[f64]) -> Result<Self, Box<HelperError>> {
        let values: VecDeque<Duration> = seconds
            .iter()
            .map(|value| Duration::try_from_secs_f64(*value).map_err(failure))
            .collect::<Result<VecDeque<Duration>, _>>()?;
        let fallback = values.back().copied().unwrap_or_default();
        Ok(Self {
            values: Arc::new(Mutex::new(values)),
            fallback,
        })
    }

    pub(super) fn handle(&self) -> Clock {
        let values = Arc::clone(&self.values);
        let fallback = self.fallback;
        Arc::new(move || {
            values
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or(fallback)
        })
    }
}

#[derive(Default)]
pub(super) struct ScriptedSleeps {
    sleeps: Mutex<Vec<Duration>>,
}

impl ScriptedSleeps {
    pub(super) fn seconds(&self) -> Vec<f64> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(Duration::as_secs_f64)
            .collect()
    }
}

#[async_trait]
impl RetrySleeper for ScriptedSleeps {
    async fn sleep(&self, duration: Duration) {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
    }
}

pub(super) struct RecordedRequest {
    pub(super) url: &'static str,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<ReplyInput>>,
    requests: Mutex<Vec<RecordedRequest>>,
    attached: Mutex<Option<Weak<NewRelicMetricsLogger>>>,
    during_send: Mutex<Option<NewRelicMetricRecord>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[ReplyInput], during_send: Option<NewRelicMetricRecord>) -> Self {
        Self {
            replies: Mutex::new(replies.iter().copied().collect()),
            requests: Mutex::new(Vec::new()),
            attached: Mutex::new(None),
            during_send: Mutex::new(during_send),
        }
    }

    pub(super) fn attach(&self, logger: &Arc<NewRelicMetricsLogger>) {
        *self.attached.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Arc::downgrade(logger));
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        let recorded = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        let mut snapshot = Vec::with_capacity(recorded.len());
        for request in recorded.iter() {
            snapshot.push(RecordedRequest {
                url: request.url,
                headers: request.headers.clone(),
                body: request.body.clone(),
            });
        }
        drop(recorded);
        snapshot
    }

    fn append_during_send(&self) {
        let queued = self
            .during_send
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(record) = queued else {
            return;
        };
        let attached = self
            .attached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(logger) = attached {
            push_record(&logger, record);
        }
    }
}

// A reqwest error cannot be built here, so a scripted network failure reports through the typed transport error

#[async_trait]
impl NewRelicTransport for ScriptedTransport {
    async fn post(&self, request: NewRelicPost) -> Result<u16, NewRelicSendError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| {
                NewRelicSendError::Request(TransportError::Rejected(HttpError::Timeout))
            })?;
        self.append_during_send();
        if reply.network {
            return Err(NewRelicSendError::Request(TransportError::Rejected(
                HttpError::Timeout,
            )));
        }
        Ok(reply.status)
    }
}
