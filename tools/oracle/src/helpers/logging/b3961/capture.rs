use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_logging::http_analytics_loggers::clock::WallClock;
use betterlite_logging::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub(crate) const SUCCESS_STATUS: u16 = 200;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Answer {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: Option<Value>,
    #[serde(default)]
    pub(crate) raise: Option<String>,
}

pub(crate) const fn default_status() -> u16 {
    SUCCESS_STATUS
}

#[derive(Clone)]
pub(crate) struct Capture {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CaptureError {
    #[error("the capture transport lock is poisoned")]
    Lock,
    #[error("the captured body is not JSON")]
    Json(#[source] serde_json::Error),
    #[error("the capture transport failed")]
    Transport,
}

impl Capture {
    pub(crate) fn with_answers(answers: Vec<Answer>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            requests: Arc::default(),
        }
    }

    fn record(&self, request: &PostRequest) -> Result<PostResponse, CaptureError> {
        let body = serde_json::from_slice::<Value>(&request.body).map_err(CaptureError::Json)?;
        let mut headers = Map::new();
        for (name, value) in &request.headers {
            if let Ok(text) = value.to_str() {
                headers.insert(name.as_str().to_owned(), Value::String(text.to_owned()));
            }
        }
        self.requests
            .lock()
            .map_err(|_poison| CaptureError::Lock)?
            .push(json!({"url": request.url.to_string(), "headers": headers, "body": body}));
        let answer = self
            .answers
            .lock()
            .map_err(|_poison| CaptureError::Lock)?
            .pop_front();
        match answer {
            Some(Answer { raise: Some(_), .. }) => Err(CaptureError::Transport),
            other => {
                let reply = other.unwrap_or(Answer {
                    status: SUCCESS_STATUS,
                    body: None,
                    raise: None,
                });
                let payload = reply.body.unwrap_or_else(|| Value::Object(Map::new()));
                let encoded = serde_json::to_vec(&payload).map_err(CaptureError::Json)?;
                Ok(PostResponse {
                    status: reply.status,
                    body: encoded,
                })
            }
        }
    }

    pub(crate) fn requests(&self) -> Result<Vec<Value>, CaptureError> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_poison| CaptureError::Lock)
    }
}

impl Delivery for Capture {
    type Error = CaptureError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        ready(self.record(&request))
    }
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<Utc>,
}

impl WallClock for FixedClock {
    fn now_utc(&self) -> DateTime<Utc> {
        self.now
    }
}
