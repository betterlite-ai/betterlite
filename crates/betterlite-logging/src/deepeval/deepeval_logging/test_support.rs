#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use super::ids::UuidSource;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Json,
}

#[derive(Clone, Debug)]
pub(crate) enum Answer {
    Reply { status: u16, body: Vec<u8> },
    Fail,
}

impl Answer {
    pub(crate) fn status(status: u16) -> Self {
        Self::Reply {
            status,
            body: Vec::new(),
        }
    }

    pub(crate) fn reply(status: u16, body: &str) -> Self {
        Self::Reply {
            status,
            body: body.as_bytes().to_vec(),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("the recording delivery is scripted to fail")]
pub(crate) struct RecordingError;

#[derive(Clone, Default)]
pub(crate) struct Recording {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    pub(crate) requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Recording {
    pub(crate) fn with_answers<I>(answers: I) -> Self
    where
        I: IntoIterator<Item = Answer>,
    {
        Self {
            answers: Arc::new(Mutex::new(answers.into_iter().collect())),
            requests: Arc::default(),
        }
    }
}

impl Delivery for Recording {
    type Error = RecordingError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let body = serde_json::from_slice(&request.body).unwrap();
        let headers = request
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect();
        self.requests.lock().unwrap().push(Recorded {
            url: request.url.to_string(),
            headers,
            body,
        });
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Answer::status(200));
        ready(match answer {
            Answer::Reply {
                status,
                body: reply_body,
            } => Ok(PostResponse {
                status,
                body: reply_body,
            }),
            Answer::Fail => Err(RecordingError),
        })
    }
}

#[derive(Default)]
pub(crate) struct CountingUuids(AtomicUsize);

impl UuidSource for CountingUuids {
    fn next_uuid(&self) -> String {
        format!("generated-{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}

pub(crate) fn at(moment: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(moment)
        .unwrap()
        .with_timezone(&Utc)
}
