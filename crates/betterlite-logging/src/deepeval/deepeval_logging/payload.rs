use betterlite_models::decisions::Json;
use serde::Deserialize;

use super::ids::UuidSource;
use super::types::{BaseApiSpan, SpanApiType, SpanTimes, SpanValue, TraceSpanApiStatus};

mod tests;

// The callback kwargs DeepEvalLogger reads: kwargs["standard_logging_object"] and kwargs["input"].
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallbackKwargs {
    #[serde(default)]
    pub standard_logging_object: Option<StandardLoggingObject>,
    #[serde(default)]
    pub input: SpanField,
}

// The standard_logging_object fields _create_base_api_span reads; error_string is spelled that way in LiteLLM.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StandardLoggingObject {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub error_string: Option<String>,
    #[serde(default)]
    pub response: Option<ResponseBody>,
}

// StandardLoggingPayload.response is declared str | list | dict | None, and only the dict carries usage and choices.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    Object(ResponseFields),
    Text(String),
    List(Vec<Json>),
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ResponseFields {
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub choices: Option<Vec<Choice>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub message: Option<Message>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub content: SpanField,
}

// A field LiteLLM reads with dict.get: an absent key takes the builder's default, a present null is used as given and exclude_none drops it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum SpanField {
    Value(SpanValue),
    Null,
    #[default]
    Absent,
}

#[must_use]
pub fn base_api_span(
    kwargs: &CallbackKwargs,
    times: SpanTimes<'_>,
    status: TraceSpanApiStatus,
    uuids: &dyn UuidSource,
) -> BaseApiSpan {
    let standard = kwargs.standard_logging_object.as_ref();
    let (input_token_count, output_token_count) = span_tokens(standard, status);
    BaseApiSpan {
        uuid: standard
            .and_then(|object| object.id.clone())
            .unwrap_or_else(|| uuids.next_uuid()),
        name: callback_name(status).to_owned(),
        status,
        span_type: SpanApiType::Llm,
        trace_uuid: standard
            .and_then(|object| object.trace_id.clone())
            .unwrap_or_else(|| uuids.next_uuid()),
        start_time: times.start.to_owned(),
        end_time: times.end.to_owned(),
        input: span_input(&kwargs.input),
        output: span_output(standard, status),
        model: standard.and_then(|object| object.model.clone()),
        input_token_count,
        output_token_count,
    }
}

const fn callback_name(status: TraceSpanApiStatus) -> &'static str {
    match status {
        TraceSpanApiStatus::Success => "litellm_success_callback",
        TraceSpanApiStatus::Errored => "litellm_failure_callback",
    }
}

fn span_input(input: &SpanField) -> Option<SpanValue> {
    match input {
        SpanField::Absent => Some(SpanValue::Text("NO_INPUT".to_owned())),
        SpanField::Null => None,
        SpanField::Value(value) => Some(value.clone()),
    }
}

fn span_output(
    standard: Option<&StandardLoggingObject>,
    status: TraceSpanApiStatus,
) -> Option<SpanValue> {
    match status {
        TraceSpanApiStatus::Success => success_output(standard),
        TraceSpanApiStatus::Errored => Some(SpanValue::Text(
            standard
                .and_then(|object| object.error_string.clone())
                .unwrap_or_default(),
        )),
    }
}

fn success_output(standard: Option<&StandardLoggingObject>) -> Option<SpanValue> {
    match first_content(standard) {
        SpanField::Absent => Some(SpanValue::Text("NO_OUTPUT".to_owned())),
        SpanField::Null => None,
        SpanField::Value(value) => Some(value),
    }
}

fn first_content(standard: Option<&StandardLoggingObject>) -> SpanField {
    let Some(response) = standard.and_then(|object| object.response.as_ref()) else {
        return SpanField::Absent;
    };
    let ResponseBody::Object(fields) = response else {
        return SpanField::Absent;
    };
    fields
        .choices
        .as_ref()
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.message.as_ref())
        .map_or(SpanField::Absent, |message| message.content.clone())
}

fn span_tokens(
    standard: Option<&StandardLoggingObject>,
    status: TraceSpanApiStatus,
) -> (Option<i64>, Option<i64>) {
    let usage = standard
        .and_then(|object| object.response.as_ref())
        .and_then(response_usage);
    match status {
        TraceSpanApiStatus::Success => (
            usage.and_then(|value| value.prompt_tokens),
            usage.and_then(|value| value.completion_tokens),
        ),
        TraceSpanApiStatus::Errored => (None, None),
    }
}

const fn response_usage(response: &ResponseBody) -> Option<Usage> {
    match response {
        ResponseBody::Object(fields) => fields.usage,
        ResponseBody::Text(_) | ResponseBody::List(_) => None,
    }
}
