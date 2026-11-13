use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_logging::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Answer {
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    body: Option<Json>,
}

const fn default_status() -> u16 {
    200
}

#[derive(Debug, Error)]
pub(super) enum ReplayError {
    #[error("the recorded request body is not JSON")]
    Body(#[source] serde_json::Error),
    #[error("a recorded header is not text")]
    Header,
    #[error("the capture state is poisoned")]
    Locked,
    #[error("the fake Braintrust endpoint answered {status}: {body}")]
    Status { status: u16, body: String },
}

#[derive(Clone, Default)]
pub(super) struct Capture {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Capture {
    pub(super) fn new(answers: &[Answer]) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers.to_vec()))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn requests(&self) -> Value {
        let requests = self
            .requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default();
        Value::Array(requests)
    }

    fn record(&self, request: &PostRequest) -> Result<PostResponse, ReplayError> {
        let body = serde_json::from_slice::<Json>(&request.body).map_err(ReplayError::Body)?;
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                let text = value.to_str().map_err(|_source| ReplayError::Header)?;
                Ok((name.as_str().to_owned(), text.to_owned()))
            })
            .collect::<Result<BTreeMap<String, String>, ReplayError>>()?;
        self.requests
            .lock()
            .map_err(|_poisoned| ReplayError::Locked)?
            .push(json!({
                "url": request.url.to_string(),
                "headers": headers,
                "body": body,
            }));
        let answer = self
            .answers
            .lock()
            .map_err(|_poisoned| ReplayError::Locked)?
            .pop_front();
        encode(answer)
    }
}

fn encode(answer: Option<Answer>) -> Result<PostResponse, ReplayError> {
    let Some(replied) = answer else {
        return Ok(PostResponse {
            status: 200,
            body: b"null".to_vec(),
        });
    };
    let encoded = replied.body.map_or_else(
        || b"null".to_vec(),
        |value| serde_json::to_vec(&value).unwrap_or_default(),
    );
    if replied.status >= 400 {
        return Err(ReplayError::Status {
            status: replied.status,
            body: String::from_utf8(encoded).unwrap_or_default(),
        });
    }
    Ok(PostResponse {
        status: replied.status,
        body: encoded,
    })
}

impl Delivery for Capture {
    type Error = ReplayError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        ready(self.record(&request))
    }
}
