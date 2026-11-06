use betterlite_models::decisions::Json;
use serde::Deserialize;

use super::ids::UuidSource;
use super::types::{
    BaseApiSpan, Environment, SpanApiType, SpanTimes, SpanValue, TraceApi, TraceSpanApiStatus,
};

mod tests;

// The callback kwargs DeepEvalLogger reads: kwargs["standard_logging_object"] and kwargs["input"].
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CallbackKwargs {
    #[serde(default)]
    pub standard_logging_object: Option<StandardLoggingObject>,
    #[serde(default)]
    pub input: Option<SpanValue>,
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
    pub content: Option<SpanValue>,
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
        input: kwargs
            .input
            .clone()
            .unwrap_or_else(|| SpanValue::Text("NO_INPUT".to_owned())),
        output: span_output(standard, status),
        model: standard.and_then(|object| object.model.clone()),
        input_token_count,
        output_token_count,
    }
}

#[must_use]
pub fn trace_api(span: BaseApiSpan, times: SpanTimes<'_>, environment: Environment) -> TraceApi {
    TraceApi {
        uuid: span.trace_uuid.clone(),
        base_spans: Vec::new(),
        agent_spans: Vec::new(),
        llm_spans: vec![span],
        retriever_spans: Vec::new(),
        tool_spans: Vec::new(),
        start_time: times.start.to_owned(),
        end_time: times.end.to_owned(),
        environment,
    }
}

const fn callback_name(status: TraceSpanApiStatus) -> &'static str {
    match status {
        TraceSpanApiStatus::Success => "litellm_success_callback",
        TraceSpanApiStatus::Errored => "litellm_failure_callback",
    }
}

fn span_output(standard: Option<&StandardLoggingObject>, status: TraceSpanApiStatus) -> SpanValue {
    match status {
        TraceSpanApiStatus::Success => {
            success_output(standard).unwrap_or_else(|| SpanValue::Text("NO_OUTPUT".to_owned()))
        }
        TraceSpanApiStatus::Errored => SpanValue::Text(
            standard
                .and_then(|object| object.error_string.clone())
                .unwrap_or_default(),
        ),
    }
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

fn success_output(standard: Option<&StandardLoggingObject>) -> Option<SpanValue> {
    let response = standard.and_then(|object| object.response.as_ref())?;
    let ResponseBody::Object(fields) = response else {
        return None;
    };
    fields
        .choices
        .as_ref()?
        .first()?
        .message
        .as_ref()?
        .content
        .clone()
}
