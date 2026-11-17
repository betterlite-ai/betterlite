use std::collections::VecDeque;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_server::pass_through_logging::tinyfish::fetch::{
    RUN_API_KEY_HEADER, RunCredentials, RunFetchFuture, RunReply, RunRequest, RunTransport,
};
use betterlite_server::pass_through_logging::tinyfish::poll::{
    PollClock, PollSleepFuture, PollSleeper,
};
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use super::fixture::{API_BASE, ScriptedReply};

struct RecordedRequest {
    url: String,
    api_key: Option<String>,
}

pub(super) struct RequestLog {
    pub count: usize,
    pub paths: Vec<String>,
    pub api_keys: Vec<Option<String>>,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<ScriptedReply>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn next_reply(&self) -> Option<ScriptedReply> {
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        if replies.len() > 1 {
            replies.pop_front()
        } else {
            replies.front().cloned()
        }
    }

    // The same first and last one LiteLLM polls with: one scripted answer repeats until the loop ends.
    pub(super) fn request_log(&self) -> RequestLog {
        let requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        let mut paths: Vec<String> = Vec::new();
        let mut api_keys: Vec<Option<String>> = Vec::new();
        for request in requests.iter() {
            let path = request
                .url
                .strip_prefix(API_BASE)
                .map_or_else(|| request.url.clone(), ToOwned::to_owned);
            if !paths.contains(&path) {
                paths.push(path);
            }
            if !api_keys.contains(&request.api_key) {
                api_keys.push(request.api_key.clone());
            }
        }
        RequestLog {
            count: requests.len(),
            paths,
            api_keys,
        }
    }
}

impl RunTransport for ScriptedTransport {
    fn get(&self, request: RunRequest) -> RunFetchFuture<'_> {
        let api_key = request
            .headers
            .get(RUN_API_KEY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: request.url,
                api_key,
            });
        let reply = self.next_reply();
        Box::pin(ready(match reply {
            Some(found) => Ok(RunReply {
                status: found.status,
                body: found.body,
            }),
            None => Err(TransportError::Rejected(HttpError::Timeout)),
        }))
    }
}

pub(super) struct FixedCredentials {
    pub(super) key: Option<SecretString>,
}

impl RunCredentials for FixedCredentials {
    fn api_key(&self) -> Option<SecretString> {
        self.key.clone()
    }
}

// LiteLLM reads time.monotonic for its absolute deadline, and the fake clock moves only when the poller sleeps.
#[derive(Default)]
pub(super) struct TestClock {
    elapsed: Mutex<Duration>,
}

impl PollClock for TestClock {
    fn now(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct AdvancingSleeper {
    pub(super) clock: Arc<TestClock>,
}

impl PollSleeper for AdvancingSleeper {
    fn sleep(&self, duration: Duration) -> PollSleepFuture<'_> {
        let mut elapsed = self
            .clock
            .elapsed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *elapsed = elapsed.saturating_add(duration);
        drop(elapsed);
        Box::pin(ready(()))
    }
}

pub(super) struct FixedSuccessClock {
    pub now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedSuccessClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}
