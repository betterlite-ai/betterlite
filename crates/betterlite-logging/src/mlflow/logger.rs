use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use super::client::{MlflowClient, MlflowError, StartRequest};
use super::model::{Kwargs, MlflowEvent, Span, SpanKind, Status};

pub struct MlflowLogger<C> {
    client: Arc<C>,
    stream_id_to_span: Mutex<HashMap<String, Span>>,
}

impl<C: MlflowClient> MlflowLogger<C> {
    pub fn new(client: C) -> Self {
        Self {
            client: Arc::new(client),
            stream_id_to_span: Mutex::new(HashMap::new()),
        }
    }

    pub fn log_success_event(
        &self,
        kwargs: &mut Kwargs,
        response: &Value,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) {
        if let Err(error) = self.handle_success(kwargs, response, start, end) {
            tracing::debug!(?error, "MLflow logging error");
        }
    }

    pub async fn async_log_success_event(
        &self,
        kwargs: &mut Kwargs,
        response: &Value,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) {
        self.log_success_event(kwargs, response, start, end);
    }

    pub fn log_failure_event(
        &self,
        kwargs: &mut Kwargs,
        response: &Value,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) {
        if let Err(error) = self.handle_failure(kwargs, response, start, end) {
            tracing::debug!(?error, "MLflow logging error");
        }
    }

    pub async fn async_log_failure_event(
        &self,
        kwargs: &mut Kwargs,
        response: &Value,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) {
        self.log_failure_event(kwargs, response, start, end);
    }

    fn handle_success(
        &self,
        kwargs: &mut Kwargs,
        response: &Value,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<(), MlflowError> {
        if bool_value(kwargs, "stream") {
            return self.handle_stream_event(kwargs, response, start, end);
        }
        let span = self.start_span_or_trace(kwargs, start)?;
        self.extract_and_set_chat_attributes(&span, kwargs, response)?;
        self.end_span_or_trace(&span, response, Status::Ok, timestamp_ns(end))
    }

    fn handle_failure(
        &self,
        kwargs: &mut Kwargs,
        response: &Value,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<(), MlflowError> {
        let span = self.start_span_or_trace(kwargs, start)?;
        if let Some(exception) = kwargs.get("exception") {
            self.client.add_event(
                &span,
                MlflowEvent {
                    name: "exception".to_owned(),
                    attributes: one_value("exception", exception.clone()),
                },
            )?;
        }
        self.extract_and_set_chat_attributes(&span, kwargs, response)?;
        self.end_span_or_trace(&span, response, Status::Error, timestamp_ns(end))
    }

    fn handle_stream_event(
        &self,
        kwargs: &mut Kwargs,
        response: &Value,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<(), MlflowError> {
        let stream_id = string_value(kwargs, "litellm_call_id");
        let span = self.tracked_or_start(&stream_id, kwargs, start)?;
        self.add_chunk_events(&span, response)?;
        let final_response = kwargs
            .get("complete_streaming_response")
            .or_else(|| kwargs.get("async_complete_streaming_response"));
        if let Some(outputs) = final_response.cloned() {
            let result = self
                .extract_and_set_chat_attributes(&span, kwargs, &outputs)
                .and_then(|()| {
                    self.end_span_or_trace(&span, &outputs, Status::Ok, timestamp_ns(end))
                });
            self.remove_stream(&stream_id)?;
            result
        } else {
            Ok(())
        }
    }

    fn tracked_or_start(
        &self,
        stream_id: &str,
        kwargs: &mut Kwargs,
        start: DateTime<Utc>,
    ) -> Result<Span, MlflowError> {
        if let Some(span) = self
            .stream_id_to_span
            .lock()
            .map_err(lock_error)?
            .get(stream_id)
            .cloned()
        {
            return Ok(span);
        }
        let span = self.start_span_or_trace(kwargs, start)?;
        let mut spans = self.stream_id_to_span.lock().map_err(lock_error)?;
        if let Some(existing) = spans.get(stream_id) {
            return Ok(existing.clone());
        }
        spans.insert(stream_id.to_owned(), span.clone());
        Ok(span)
    }

    fn remove_stream(&self, stream_id: &str) -> Result<(), MlflowError> {
        self.stream_id_to_span
            .lock()
            .map_err(lock_error)?
            .remove(stream_id);
        Ok(())
    }

    fn add_chunk_events(&self, span: &Span, response: &Value) -> Result<(), MlflowError> {
        let choices = response
            .get("choices")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for choice in choices {
            if let Some(delta) = choice.get("delta") {
                // LiteLLM passes the bound model_dump method to json.dumps; debug text preserves that quirk.
                let method_reference = Value::String(format!("{delta:?}"));
                self.client
                    .add_event(span, MlflowEvent::streaming_chunk(method_reference))?;
            }
        }
        Ok(())
    }

    fn extract_and_set_chat_attributes(
        &self,
        span: &Span,
        kwargs: &mut Kwargs,
        response: &Value,
    ) -> Result<(), MlflowError> {
        let inputs = construct_input(kwargs);
        let mut messages = inputs
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(choices) = response.get("choices").and_then(Value::as_array) {
            messages.extend(
                choices
                    .iter()
                    .filter_map(|choice| choice.get("message").cloned()),
            );
        }
        if !messages.is_empty() {
            self.client.add_event(
                span,
                MlflowEvent {
                    name: "chat_messages".to_owned(),
                    attributes: one_value("messages", Value::Array(messages)),
                },
            )?;
        }
        if let Some(tools) = inputs.get("tools") {
            self.client.add_event(
                span,
                MlflowEvent {
                    name: "chat_tools".to_owned(),
                    attributes: one_value("tools", tools.clone()),
                },
            )?;
        }
        Ok(())
    }

    fn start_span_or_trace(
        &self,
        kwargs: &mut Kwargs,
        start: DateTime<Utc>,
    ) -> Result<Span, MlflowError> {
        let call_type = string_value_or(kwargs, "call_type", "completion");
        let request = StartRequest {
            name: format!("litellm-{call_type}"),
            kind: span_kind(&call_type),
            inputs: construct_input(kwargs),
            attributes: extract_attributes(kwargs),
            tags: transform_tags(kwargs.get("request_tags")),
            start_time_ns: timestamp_ns(start),
        };
        if let Some(active) = self.client.active_span()? {
            self.client
                .start_span(request, &active.request_id, &active.span_id)
        } else {
            self.client.start_trace(request)
        }
    }

    fn end_span_or_trace(
        &self,
        span: &Span,
        outputs: &Value,
        status: Status,
        end_time_ns: i64,
    ) -> Result<(), MlflowError> {
        if span.parent_id.is_some() {
            self.client.end_span(span, outputs, status, end_time_ns)
        } else {
            self.client.end_trace(span, outputs, status, end_time_ns)
        }
    }
}

fn timestamp_ns(time: DateTime<Utc>) -> i64 {
    time.timestamp_nanos_opt().unwrap_or(i64::MAX)
}
fn lock_error<T>(_: std::sync::PoisonError<T>) -> MlflowError {
    MlflowError::Operation("MLflow stream lock poisoned".to_owned())
}
fn bool_value(values: &Kwargs, key: &str) -> bool {
    values.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn string_value(values: &Kwargs, key: &str) -> String {
    values
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn string_value_or(values: &Kwargs, key: &str, fallback: &str) -> String {
    values
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_owned()
}
fn one_value(key: &str, value: Value) -> Kwargs {
    let mut values = Map::new();
    values.insert(key.to_owned(), value);
    values
}
fn span_kind(call_type: &str) -> SpanKind {
    if call_type == "embeddings" {
        SpanKind::Embedding
    } else {
        SpanKind::Llm
    }
}

fn construct_input(kwargs: &mut Kwargs) -> Kwargs {
    let mut inputs = one_value(
        "messages",
        kwargs.get("messages").cloned().unwrap_or(Value::Null),
    );
    if let Some(tools) = kwargs.get("tools").filter(|value| !value.is_null()) {
        inputs.insert("tools".to_owned(), tools.clone());
    }
    if let Some(optional) = kwargs
        .get_mut("optional_params")
        .and_then(Value::as_object_mut)
    {
        for key in ["functions", "tools", "stream", "tool_choice", "user"] {
            if let Some(value) = optional.remove(key) {
                if !value.is_null() {
                    inputs.insert(key.to_owned(), value);
                }
            }
        }
    }
    if let Some(prediction) = kwargs.get("prediction").filter(|value| !value.is_null()) {
        inputs.insert("prediction".to_owned(), prediction.clone());
    }
    inputs
}

fn extract_attributes(kwargs: &Kwargs) -> Kwargs {
    let mut attributes = Kwargs::new();
    for key in ["litellm_call_id", "call_type", "model"] {
        attributes.insert(
            key.to_owned(),
            kwargs.get(key).cloned().unwrap_or(Value::Null),
        );
    }
    if let Some(standard) = kwargs
        .get("standard_logging_object")
        .and_then(Value::as_object)
    {
        for key in [
            "api_base",
            "cache_hit",
            "response_cost",
            "saved_cache_cost",
            "request_tags",
            "response",
        ] {
            attributes.insert(
                key.to_owned(),
                standard.get(key).cloned().unwrap_or(Value::Null),
            );
        }
        let mut usage = Kwargs::new();
        for (source, target) in [
            ("prompt_tokens", "input_tokens"),
            ("completion_tokens", "output_tokens"),
            ("total_tokens", "total_tokens"),
        ] {
            usage.insert(
                target.to_owned(),
                standard.get(source).cloned().unwrap_or(Value::Null),
            );
        }
        attributes.insert("mlflow.chat.tokenUsage".to_owned(), Value::Object(usage));
    } else {
        attributes.insert(
            "cache_hit".to_owned(),
            kwargs.get("cache_hit").cloned().unwrap_or(Value::Null),
        );
        attributes.insert(
            "custom_llm_provider".to_owned(),
            kwargs
                .get("custom_llm_provider")
                .cloned()
                .unwrap_or(Value::Null),
        );
        attributes.insert(
            "api_base".to_owned(),
            kwargs
                .get("litellm_params")
                .and_then(Value::as_object)
                .and_then(|value| value.get("api_base"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        attributes.insert(
            "response_cost".to_owned(),
            kwargs.get("response_cost").cloned().unwrap_or(Value::Null),
        );
    }
    attributes
}

fn transform_tags(value: Option<&Value>) -> Kwargs {
    let mut tags = Kwargs::new();
    if let Some(values) = value.and_then(Value::as_array) {
        for tag in values.iter().filter_map(Value::as_str) {
            let (key, value) = tag
                .split_once(':')
                .map_or((tag, ""), |(key, value)| (key, value));
            tags.insert(
                key.trim().to_owned(),
                Value::String(value.trim().to_owned()),
            );
        }
    }
    tags
}
