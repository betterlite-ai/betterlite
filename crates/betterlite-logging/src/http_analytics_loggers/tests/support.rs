use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::http_analytics_loggers::clock::WallClock;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::http_analytics_loggers::event::Event;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Json,
}

#[derive(Clone, Debug)]
pub(crate) enum Answer {
    Ok(Json),
    Status(u16),
    Fail,
}

#[derive(Debug, thiserror::Error)]
#[error("the capture transport failed")]
pub(crate) struct CaptureError;

#[derive(Clone, Default)]
pub(crate) struct Capture {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Capture {
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
        self.requests.lock().unwrap().clone()
    }
}

impl Delivery for Capture {
    type Error = CaptureError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let parsed = serde_json::from_slice::<Json>(&request.body);
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
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Answer::Ok(Json::Object(IndexMap::new())));
        let Ok(decoded) = parsed else {
            return ready(Err(CaptureError));
        };
        self.requests.lock().unwrap().push(Recorded {
            url: request.url.to_string(),
            headers,
            body: decoded,
        });
        ready(match answer {
            Answer::Ok(value) => match serde_json::to_vec(&value) {
                Ok(encoded) => Ok(PostResponse {
                    status: 200,
                    body: encoded,
                }),
                Err(_source) => Err(CaptureError),
            },
            Answer::Status(status) => Ok(PostResponse {
                status,
                body: Vec::new(),
            }),
            Answer::Fail => Err(CaptureError),
        })
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

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("lock"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(crate) fn subscribe(&self) -> DefaultGuard {
        let writer = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .finish();
        set_default(subscriber)
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

// the fixed point of litellm.ModelResponse.model_dump() for the response every case sends
pub(crate) const COMPLETION: &str = r#"{"id":"chatcmpl-1","created":1700000000,"model":"gpt-4o","object":"chat.completion","system_fingerprint":null,"choices":[{"finish_reason":"stop","index":0,"message":{"content":"hi","role":"assistant","tool_calls":null,"function_call":null}}],"usage":{"completion_tokens":2,"prompt_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null}}"#;

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn object(value: &Json) -> &IndexMap<String, Json> {
    let Json::Object(fields) = value else {
        panic!("the fixture is a JSON object");
    };
    fields
}

pub(crate) fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    object(value)
        .get(name)
        .unwrap_or_else(|| panic!("missing field {name}"))
}

pub(crate) fn time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("the fixture time is RFC 3339")
        .with_timezone(&Utc)
}

pub(crate) fn event<'call>(
    kwargs: &'call Json,
    response: Option<&'call Json>,
    start: &str,
    end: &str,
) -> Event<'call> {
    Event {
        kwargs,
        response,
        start_time: time(start),
        end_time: time(end),
    }
}
