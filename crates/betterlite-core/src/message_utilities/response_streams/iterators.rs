use std::future::Future as _;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::park_timeout;
use std::time::Duration;

use betterlite_models::decisions::Json;
use futures_util::Stream;
use indexmap::IndexMap;
use tokio::time::{Sleep, sleep};

use super::StreamError;
use crate::message_utilities::json;

pub struct DeltaIdentity<'identity> {
    pub id: &'identity str,
    pub created: i64,
}

pub struct ModelResponseIterator {
    chunk: Option<Json>,
}

impl ModelResponseIterator {
    pub fn new(response: Json, identity: Option<DeltaIdentity<'_>>) -> Result<Self, StreamError> {
        let chunk = match identity {
            Some(found) => Some(delta_chunk(&response, &found)?),
            None => Some(response),
        };
        Ok(Self { chunk })
    }
}

impl Iterator for ModelResponseIterator {
    type Item = Json;

    fn next(&mut self) -> Option<Json> {
        self.chunk.take()
    }
}

impl Stream for ModelResponseIterator {
    type Item = Json;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Json>> {
        Poll::Ready(self.get_mut().chunk.take())
    }
}

// LiteLLM builds a fresh ModelResponseStream and copies choices[0].message.content into its delta
pub fn delta_chunk(response: &Json, identity: &DeltaIdentity<'_>) -> Result<Json, StreamError> {
    let content = message_content(response)?;
    let mut delta = IndexMap::new();
    delta.insert(String::from("content"), content);
    delta.insert(String::from("role"), Json::Null);
    delta.insert(String::from("function_call"), Json::Null);
    delta.insert(String::from("tool_calls"), Json::Null);
    delta.insert(String::from("audio"), Json::Null);
    let mut choice = IndexMap::new();
    choice.insert(String::from("finish_reason"), Json::Null);
    choice.insert(
        String::from("index"),
        Json::Number(serde_json::Number::from(0_u8)),
    );
    choice.insert(String::from("delta"), Json::Object(delta));
    choice.insert(String::from("logprobs"), Json::Null);
    let mut chunk = IndexMap::new();
    chunk.insert(String::from("id"), Json::Text(identity.id.to_owned()));
    chunk.insert(
        String::from("created"),
        Json::Number(serde_json::Number::from(identity.created)),
    );
    chunk.insert(
        String::from("object"),
        Json::Text(String::from("chat.completion.chunk")),
    );
    chunk.insert(
        String::from("choices"),
        Json::List(vec![Json::Object(choice)]),
    );
    chunk.insert(String::from("provider_specific_fields"), Json::Null);
    Ok(Json::Object(chunk))
}

fn message_content(response: &Json) -> Result<Json, StreamError> {
    let fields = json::object(response).ok_or(StreamError::NotAnObject)?;
    let Some(Json::List(choices)) = fields.get("choices") else {
        return Err(StreamError::NoChoices);
    };
    let message = choices
        .first()
        .and_then(json::object)
        .and_then(|choice| choice.get("message"))
        .and_then(json::object)
        .ok_or(StreamError::NoMessage)?;
    Ok(message.get("content").cloned().unwrap_or(Json::Null))
}

pub struct ModelResponseListIterator {
    responses: Vec<Json>,
    index: usize,
    delay: Option<Duration>,
    sleeping: Option<Pin<Box<Sleep>>>,
}

impl ModelResponseListIterator {
    pub fn new(responses: Vec<Json>, delay_seconds: Option<f64>) -> Result<Self, StreamError> {
        let delay = match delay_seconds {
            Some(seconds) if seconds != 0.0_f64 => {
                let parsed = Duration::try_from_secs_f64(seconds)
                    .map_err(|_source| StreamError::Delay { seconds })?;
                Some(parsed)
            }
            _ => None,
        };
        Ok(Self {
            responses,
            index: 0,
            delay,
            sleeping: None,
        })
    }

    fn wait_for_delay(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let Some(delay) = self.delay else {
            return Poll::Ready(());
        };
        let timer = self.sleeping.get_or_insert_with(|| Box::pin(sleep(delay)));
        if timer.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        self.sleeping = None;
        Poll::Ready(())
    }

    fn take_next(&mut self) -> Poll<Option<Json>> {
        let Some(found) = self.responses.get(self.index) else {
            return Poll::Ready(None);
        };
        let response = found.clone();
        self.index = self.index.saturating_add(1);
        Poll::Ready(Some(response))
    }
}

impl Iterator for ModelResponseListIterator {
    type Item = Json;

    fn next(&mut self) -> Option<Json> {
        let response = self.responses.get(self.index)?.clone();
        self.index = self.index.saturating_add(1);
        if let Some(delay) = self.delay {
            park_timeout(delay);
        }
        Some(response)
    }
}

impl Stream for ModelResponseListIterator {
    type Item = Json;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Json>> {
        let this = self.get_mut();
        if this.index >= this.responses.len() {
            return Poll::Ready(None);
        }
        if this.wait_for_delay(cx).is_pending() {
            return Poll::Pending;
        }
        this.take_next()
    }
}
