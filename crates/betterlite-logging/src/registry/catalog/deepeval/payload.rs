use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::deepeval::deepeval_logging::payload::{
    CallbackKwargs, Choice, Message, ResponseBody, ResponseFields, SpanField,
    StandardLoggingObject, Usage,
};
use crate::deepeval::deepeval_logging::types::SpanValue;
use crate::fields::payload::StandardLoggingPayload;

// DeepevalLogger reads kwargs["standard_logging_object"] and kwargs["input"], and the logging frame puts the request messages in input
pub(super) fn callback_kwargs(payload: &StandardLoggingPayload) -> CallbackKwargs {
    CallbackKwargs {
        standard_logging_object: Some(StandardLoggingObject {
            id: Some(payload.id.clone()),
            trace_id: Some(payload.trace_id.clone()),
            model: Some(payload.model.clone()),
            error_str: payload.error_str.clone(),
            response: response_body(payload.response.as_ref()),
        }),
        input: span_field(payload.messages.as_ref()),
    }
}

// StandardLoggingPayload.response is declared str | list | dict | None, and only the dict carries usage and choices
fn response_body(response: Option<&Json>) -> Option<ResponseBody> {
    match response {
        Some(Json::Text(text)) => Some(ResponseBody::Text(text.clone())),
        Some(Json::List(items)) => Some(ResponseBody::List(items.clone())),
        Some(Json::Object(fields)) => Some(ResponseBody::Object(response_fields(fields))),
        Some(Json::Null | Json::Bool(_) | Json::Number(_)) | None => None,
    }
}

fn response_fields(fields: &IndexMap<String, Json>) -> ResponseFields {
    ResponseFields {
        usage: fields.get("usage").map(usage),
        choices: choices(fields.get("choices")),
    }
}

fn usage(value: &Json) -> Usage {
    let Json::Object(fields) = value else {
        return Usage::default();
    };
    Usage {
        prompt_tokens: integer(fields.get("prompt_tokens")),
        completion_tokens: integer(fields.get("completion_tokens")),
    }
}

fn integer(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn choices(value: Option<&Json>) -> Option<Vec<Choice>> {
    let Some(Json::List(items)) = value else {
        return None;
    };
    Some(items.iter().map(choice).collect())
}

fn choice(item: &Json) -> Choice {
    let Json::Object(fields) = item else {
        return Choice::default();
    };
    Choice {
        message: fields.get("message").map(message),
    }
}

fn message(value: &Json) -> Message {
    let Json::Object(fields) = value else {
        return Message::default();
    };
    Message {
        content: span_field(fields.get("content")),
    }
}

fn span_field(value: Option<&Json>) -> SpanField {
    match value {
        Some(Json::Null) => SpanField::Null,
        Some(Json::Text(text)) => SpanField::Value(SpanValue::Text(text.clone())),
        Some(Json::Object(fields)) => SpanField::Value(SpanValue::Object(fields.clone())),
        Some(Json::List(items)) => SpanField::Value(SpanValue::List(items.clone())),
        Some(Json::Bool(_) | Json::Number(_)) | None => SpanField::Absent,
    }
}
