use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::super::ids::IdSource;
use super::super::payload::CallbackKwargs;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};

#[derive(Clone, Debug)]
pub(crate) enum Answer {
    Reply { status: u16, body: String },
    Fail,
}

impl Answer {
    pub(crate) fn status(status: u16) -> Self {
        Self::Reply {
            status,
            body: String::new(),
        }
    }

    pub(crate) fn reply(status: u16, body: &str) -> Self {
        Self::Reply {
            status,
            body: body.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Json,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("the recording delivery is scripted to fail")]
pub(crate) struct RecordingError;

#[derive(Clone, Default)]
pub(crate) struct Recording {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
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

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.to_string(),
                headers,
                body,
            });
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Answer::status(200));
        ready(match answer {
            Answer::Reply {
                status,
                body: reply_body,
            } => Ok(PostResponse {
                status,
                body: reply_body.into_bytes(),
            }),
            Answer::Fail => Err(RecordingError),
        })
    }
}

#[derive(Default)]
pub(crate) struct FixedIds(AtomicUsize);

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        format!("generated-{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}

pub(crate) fn at(moment: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(moment)
        .unwrap()
        .with_timezone(&Utc)
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn kwargs(text: &str) -> CallbackKwargs {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn variables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn object(value: &Json) -> indexmap::IndexMap<String, Json> {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.clone()
}

pub(crate) fn text(value: &Json) -> &str {
    let Json::Text(text) = value else {
        panic!("the value is text");
    };
    text
}

#[derive(Clone, Default)]
pub(crate) struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    pub(crate) fn contents(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture(buffer: &Buffer) -> DefaultGuard {
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    set_default(subscriber)
}
