use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use betterlite_azure::image_transport::request::{ImageClientConstructor, TimeoutPhases};
use betterlite_azure::image_transport::{ImageClock, ImageReply, ImageRequest, ImageTransport};
use betterlite_http::proxy::transport_error::TransportError;
use serde_json::Value;

use super::input::Reply;
use super::ops::header_map;

pub(super) struct Fixture {
    pub(super) replies: Mutex<VecDeque<Reply>>,
    pub(super) calls: Mutex<Vec<RecordedCall>>,
    pub(super) constructed: Mutex<Vec<TimeoutPhases>>,
    pub(super) clock: Arc<FixtureClock>,
}

pub(super) struct RecordedCall {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Option<Value>,
}

#[derive(Clone)]
pub(super) struct FixtureTransport {
    pub(super) state: Arc<Fixture>,
}

impl ImageTransport for FixtureTransport {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        let body = (!request.body.is_empty())
            .then(|| serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null));
        lock(&self.state.calls).push(RecordedCall {
            method: request.method.to_string(),
            url: request.url.clone(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
            body,
        });
        let Some(reply) = lock(&self.state.replies).pop_front() else {
            return ready(Err(TransportError::Closed));
        };
        if reply.delay > 0 {
            self.state
                .clock
                .elapsed
                .fetch_add(reply.delay, Ordering::SeqCst);
        }
        ready(Ok(ImageReply {
            status: reply.status,
            headers: header_map(&reply.headers).unwrap_or_default(),
            body: reply.body.into_bytes(),
        }))
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Default)]
pub(super) struct FixtureClock {
    pub(super) elapsed: AtomicU64,
    pub(super) sleeps: Mutex<Vec<i64>>,
}

impl ImageClock for FixtureClock {
    fn now(&self) -> Duration {
        Duration::from_secs(self.elapsed.load(Ordering::SeqCst))
    }

    fn pause(&self, seconds: i64) -> impl Future<Output = ()> + Send {
        lock(&self.sleeps).push(seconds);
        if let Ok(positive) = u64::try_from(seconds) {
            self.elapsed.fetch_add(positive, Ordering::SeqCst);
        }
        ready(())
    }
}

pub(super) struct Constructor {
    pub(super) state: Arc<Fixture>,
}

impl ImageClientConstructor for Constructor {
    type Client = FixtureTransport;

    fn construct(&self, timeout: TimeoutPhases) -> FixtureTransport {
        lock(&self.state.constructed).push(timeout);
        FixtureTransport {
            state: Arc::clone(&self.state),
        }
    }
}
