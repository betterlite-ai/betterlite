use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::response_events::event_decode::DecodedStreamResponse;
use betterlite_openai::response_events::{ReplyBody, ResponsesReply};
use serde_json::{Value, json};

pub(super) fn usage_json(usage: Option<&ResponseApiUsage>) -> Value {
    let Some(reported) = usage else {
        return json!({"cost": Value::Null, "ticks": Value::Null});
    };
    let cost = reported.cost.map(|cost| cost.normalize().to_string());
    let ticks = reported
        .extra
        .get("cost_in_usd_ticks")
        .and_then(|ticks| serde_json::to_value(ticks).ok())
        .unwrap_or(Value::Null);
    json!({"cost": cost, "ticks": ticks})
}

// every response.* event carries the usage LiteLLM serializes, and only the terminal three restate its cost
pub(super) const fn known_usage(record: &KnownResponseEvent) -> Option<&ResponseApiUsage> {
    if let KnownResponseEvent::ResponseCreated(fields)
    | KnownResponseEvent::ResponseInProgress(fields)
    | KnownResponseEvent::ResponseCompleted(fields)
    | KnownResponseEvent::ResponseIncomplete(fields)
    | KnownResponseEvent::ResponseFailed(fields) = record
    {
        return fields.response.usage.as_ref();
    }
    None
}

pub(super) fn event_value(event: &DecodedStreamResponse) -> Value {
    let usage = match event {
        DecodedStreamResponse::Known { record, .. } => known_usage(record),
        DecodedStreamResponse::Generic(_) | DecodedStreamResponse::Lax { .. } => None,
    };
    let mut value = usage_json(usage);
    if let Value::Object(fields) = &mut value {
        fields.insert(
            String::from("class"),
            Value::String(event.class().class_name().to_owned()),
        );
    }
    value
}

pub(super) fn reply_usage(reply: &ResponsesReply) -> Value {
    match &reply.response {
        ReplyBody::Typed(response) => usage_json(response.usage.as_ref()),
        ReplyBody::Lax(_fields) => usage_json(None),
    }
}

pub(super) fn error_status(error: &OpenAiError) -> Option<u16> {
    if let OpenAiError::Provider(provider) = error {
        Some(provider.status_code)
    } else {
        None
    }
}
