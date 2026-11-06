use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use thiserror::Error;

use crate::braintrust_logging::logger::BraintrustLogger;
use crate::braintrust_logging::payload::{ResponseObject, SuccessEvent};
use crate::braintrust_logging::settings::Settings;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};

pub(crate) const KEY: &str = "braintrust-test-key";
pub(crate) const BASE: &str = "https://braintrust.test/v1";

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn at_path<'json>(value: &'json Json, path: &[&str]) -> &'json Json {
    let mut current = value;
    for step in path {
        current = match current {
            Json::Object(fields) => fields.get(*step).unwrap_or(&Json::Null),
            Json::List(items) => &items[step.parse::<usize>().unwrap()],
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
                panic!("cannot descend into a scalar at {step}");
            }
        };
    }
    current
}

pub(crate) fn posted(replay: &Replay, index: usize, path: &[&str]) -> Json {
    at_path(&replay.requests()[index].body, path).clone()
}

pub(crate) fn posted_event(replay: &Replay, index: usize) -> Json {
    posted(replay, index, &["events", "0"])
}

pub(crate) fn at(seconds: i64, nanos: u32) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, nanos).unwrap()
}

pub(crate) fn settings(default_tags: &[&str]) -> Settings {
    Settings {
        api_key: SecretString::from(KEY),
        api_base: BASE.to_owned(),
        default_tags: default_tags.iter().map(|tag| (*tag).to_owned()).collect(),
    }
}

#[derive(Debug, Error)]
pub(crate) enum ReplayError {
    #[error("the recorded request body is not JSON")]
    Body(#[source] serde_json::Error),
    #[error("a recorded header is not text")]
    Header,
    #[error("the replay state is poisoned")]
    Locked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedRequest {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Json,
}

#[derive(Debug, Clone)]
pub(crate) struct Answer {
    status: u16,
    body: Option<Json>,
}

pub(crate) fn answer(body: &str) -> Answer {
    Answer {
        status: 200,
        body: Some(json(body)),
    }
}

pub(crate) fn failing(status: u16, body: &str) -> Answer {
    Answer {
        status,
        body: Some(json(body)),
    }
}

#[derive(Clone, Default)]
pub(crate) struct Replay {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl Replay {
    pub(crate) fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
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
            .push(RecordedRequest {
                url: request.url.to_string(),
                headers,
                body,
            });
        let replied = self
            .answers
            .lock()
            .map_err(|_poisoned| ReplayError::Locked)?
            .pop_front()
            .unwrap_or(Answer {
                status: 200,
                body: None,
            });
        let response_body = replied.body.map_or_else(
            || b"null".to_vec(),
            |value| serde_json::to_vec(&value).unwrap(),
        );
        Ok(PostResponse {
            status: replied.status,
            body: response_body,
        })
    }
}

impl Delivery for Replay {
    type Error = ReplayError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        ready(self.record(&request))
    }
}

pub(crate) fn logger(
    answers: Vec<Answer>,
    default_tags: &[&str],
) -> (BraintrustLogger<Replay>, Replay) {
    let replay = Replay::new(answers);
    let logger = BraintrustLogger::new(settings(default_tags), replay.clone());
    (logger, replay)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseKind {
    None,
    Model,
    TextCompletion,
    Embedding,
    Image,
}

pub(crate) struct Scenario {
    pub(crate) call_id: Json,
    pub(crate) messages: Json,
    pub(crate) standard_logging_object: Json,
    pub(crate) metadata: Json,
    pub(crate) response_cost: Json,
    pub(crate) call_type: Option<String>,
    pub(crate) response_kind: ResponseKind,
    pub(crate) response_body: Json,
    pub(crate) api_call_start_time: Option<DateTime<Utc>>,
    pub(crate) completion_start_time: Option<DateTime<Utc>>,
    pub(crate) start_time: DateTime<Utc>,
    pub(crate) end_time: DateTime<Utc>,
}

impl Scenario {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            call_id: Json::Null,
            messages: Json::Null,
            standard_logging_object: Json::Object(indexmap::IndexMap::new()),
            metadata: Json::Object(indexmap::IndexMap::new()),
            response_cost: Json::Null,
            call_type: None,
            response_kind: ResponseKind::None,
            response_body: Json::Null,
            api_call_start_time: None,
            completion_start_time: None,
            start_time: at(1_700_000_000, 0),
            end_time: at(1_700_000_001, 500_000_000),
        }
    }

    #[cfg(test)]
    pub(crate) fn event(&self) -> SuccessEvent<'_> {
        let response = match self.response_kind {
            ResponseKind::None => None,
            ResponseKind::Model => Some(ResponseObject::Model(&self.response_body)),
            ResponseKind::TextCompletion => {
                Some(ResponseObject::TextCompletion(&self.response_body))
            }
            ResponseKind::Embedding => Some(ResponseObject::Embedding(&self.response_body)),
            ResponseKind::Image => Some(ResponseObject::Image(&self.response_body)),
        };
        SuccessEvent {
            call_id: Some(&self.call_id),
            messages: Some(&self.messages),
            standard_logging_object: Some(&self.standard_logging_object),
            metadata: Some(&self.metadata),
            response_cost: Some(&self.response_cost),
            call_type: self.call_type.as_deref(),
            response,
            api_call_start_time: self.api_call_start_time,
            completion_start_time: self.completion_start_time,
            start_time: self.start_time,
            end_time: self.end_time,
        }
    }
}

pub(crate) fn model_completion() -> Json {
    json(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}"#,
    )
}
