use std::collections::{BTreeMap, HashMap};
use std::fmt::{self, Write as _};
use std::future::{Ready, ready};
use std::sync::Mutex;

use super::attributes::{Details, Kwargs, transform_tags};
use super::client::{self, Client, EndRequest, StartRequest};
use super::model::{Event, Response, Span, SpanKind, Status, Timestamp};

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("MLflow client failed")]
    Client(#[from] client::ClientError),
    #[error("MLflow streaming state lock poisoned")]
    Lock,
    #[error("MLflow streaming capacity exhausted or zero")]
    Capacity,
    #[error("MLflow timestamp is outside the nanosecond range")]
    Timestamp,
    #[error("MLflow response choice lacks a message or bound delta method")]
    Choice,
    #[error("MLflow input messages are missing")]
    Messages,
    #[error("MLflow delta serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("MLflow delta formatting failed")]
    Formatting(#[from] fmt::Error),
}

pub struct Call<'event, Data> {
    pub kwargs: &'event mut Kwargs<Data>,
    pub response: &'event Response<Data>,
    pub start: Timestamp,
    pub end: Timestamp,
}

pub struct Logger<C> {
    client: C,
    streams: Mutex<HashMap<Option<String>, Span>>,
    max_streams: usize,
}

impl<C: Client> Logger<C> {
    pub fn new(client: C, max_streams: usize) -> Result<Self, LoggingError> {
        if max_streams == 0 {
            return Err(LoggingError::Capacity);
        }
        Ok(Self {
            client,
            streams: Mutex::new(HashMap::new()),
            max_streams,
        })
    }

    pub fn log_success_event(&self, call: Call<'_, C::Data>) {
        if let Err(error) = self.handle(call, Status::Ok) {
            tracing::debug!(?error, "MLflow logging error");
        }
    }

    pub fn async_log_success_event(&self, call: Call<'_, C::Data>) -> Ready<()> {
        self.log_success_event(call);
        ready(())
    }

    pub fn log_failure_event(&self, call: Call<'_, C::Data>) {
        if let Err(error) = self.handle(call, Status::Error) {
            tracing::debug!(?error, "MLflow logging error");
        }
    }

    pub fn async_log_failure_event(&self, call: Call<'_, C::Data>) -> Ready<()> {
        self.log_failure_event(call);
        ready(())
    }

    fn handle(&self, call: Call<'_, C::Data>, status: Status) -> Result<(), LoggingError> {
        let Call {
            kwargs,
            response,
            start,
            end,
        } = call;
        if status == Status::Ok && kwargs.stream {
            return self.handle_stream(kwargs, response, start, end);
        }
        let span = self.start(kwargs, start)?;
        if status == Status::Error
            && let Some(exception) = &kwargs.exception
        {
            self.client
                .add_event(&span, Event::Exception(exception.clone()))?;
        }
        self.set_chat(&span, kwargs, response)?;
        self.end(&span, response, status, end)
    }

    fn handle_stream(
        &self,
        kwargs: &mut Kwargs<C::Data>,
        response: &Response<C::Data>,
        start: Timestamp,
        end: Timestamp,
    ) -> Result<(), LoggingError> {
        let stream_id = kwargs.litellm_call_id.clone();
        let span = self.tracked_or_start(kwargs, start)?;
        if let Err(error) = self.add_chunks(&span, response) {
            tracing::debug!(?error, "Error adding chunk events to span");
        }
        let final_response = kwargs
            .complete_streaming_response
            .as_ref()
            .or(kwargs.async_complete_response.as_ref())
            .cloned();
        let Some(outputs) = final_response else {
            return Ok(());
        };
        let result = self
            .set_chat(&span, kwargs, &outputs)
            .and_then(|()| self.end(&span, &outputs, Status::Ok, end));
        self.streams
            .lock()
            .map_err(|_poisoned| LoggingError::Lock)?
            .remove(&stream_id);
        result
    }

    fn tracked_or_start(
        &self,
        kwargs: &mut Kwargs<C::Data>,
        start: Timestamp,
    ) -> Result<Span, LoggingError> {
        let existing = self
            .streams
            .lock()
            .map_err(|_poisoned| LoggingError::Lock)?
            .get(&kwargs.litellm_call_id)
            .cloned();
        if let Some(span) = existing {
            return Ok(span);
        }
        let mut streams = self
            .streams
            .lock()
            .map_err(|_poisoned| LoggingError::Lock)?;
        if let Some(span) = streams.get(&kwargs.litellm_call_id) {
            return Ok(span.clone());
        }
        if streams.len() >= self.max_streams {
            return Err(LoggingError::Capacity);
        }
        let span = self.start(kwargs, start)?;
        streams.insert(kwargs.litellm_call_id.clone(), span.clone());
        drop(streams);
        Ok(span)
    }

    fn add_chunks(&self, span: &Span, response: &Response<C::Data>) -> Result<(), LoggingError> {
        for choice in &response.choices {
            let method = choice.delta_method.as_ref().ok_or(LoggingError::Choice)?;
            let delta = dump_method(method)?;
            self.client
                .add_event(span, Event::StreamingChunk { delta })?;
        }
        Ok(())
    }

    fn set_chat(
        &self,
        span: &Span,
        kwargs: &mut Kwargs<C::Data>,
        response: &Response<C::Data>,
    ) -> Result<(), LoggingError> {
        if !self.client.chat_available() {
            return Ok(());
        }
        let inputs = kwargs.construct_input();
        let mut messages = inputs.messages.ok_or(LoggingError::Messages)?;
        for choice in &response.choices {
            messages.push(choice.message.clone().ok_or(LoggingError::Choice)?);
        }
        if !messages.is_empty() {
            self.client.set_chat_messages(span, messages)?;
        }
        if let Some(tools) = inputs.tools {
            self.client.set_chat_tools(span, tools)?;
        }
        Ok(())
    }

    fn start(&self, kwargs: &mut Kwargs<C::Data>, start: Timestamp) -> Result<Span, LoggingError> {
        let call_type = kwargs.call_type.as_deref().unwrap_or("completion");
        let name = format!("litellm-{call_type}");
        let kind = if call_type == "embeddings" {
            SpanKind::Embedding
        } else {
            SpanKind::Llm
        };
        let inputs = kwargs.construct_input();
        let attributes = kwargs.extract_attributes();
        let parent = self.client.active_span()?;
        let tags = match (&parent, &attributes.details) {
            (None, Details::Standard { request_tags, .. }) => transform_tags(request_tags),
            (Some(_), Details::Standard { .. }) | (_, Details::Fallback { .. }) => BTreeMap::new(),
        };
        let request = StartRequest {
            name,
            kind,
            inputs,
            attributes,
            tags,
            start_time_ns: start.timestamp_nanos_opt().ok_or(LoggingError::Timestamp)?,
        };
        if let Some(active) = parent {
            Ok(self.client.start_span(request, &active)?)
        } else {
            Ok(self.client.start_trace(request)?)
        }
    }

    fn end(
        &self,
        span: &Span,
        outputs: &Response<C::Data>,
        status: Status,
        end: Timestamp,
    ) -> Result<(), LoggingError> {
        let request = EndRequest {
            outputs,
            status,
            end_time_ns: end.timestamp_nanos_opt().ok_or(LoggingError::Timestamp)?,
        };
        if span.parent_id.is_some() {
            self.client.end_span(span, request)?;
        } else {
            self.client.end_trace(span, request)?;
        }
        Ok(())
    }
}

fn dump_method(method: &str) -> Result<String, LoggingError> {
    let encoded = serde_json::to_string(method)?;
    let mut ascii = String::new();
    for character in encoded.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else {
            for unit in character.encode_utf16(&mut [0; 2]) {
                write!(ascii, "\\u{unit:04x}")?;
            }
        }
    }
    Ok(ascii)
}
