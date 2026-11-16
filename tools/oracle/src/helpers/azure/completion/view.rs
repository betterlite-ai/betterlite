use betterlite_azure::completion::error::AzureTextError;
use betterlite_azure::completion::response::ChatReply;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::error::TextCompletionError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use serde_json::Number;

use super::hooks::Recorder;
use super::source::ScriptedSource;

const CREATED_FLOOR: i64 = 1_000_000_000;

pub(super) struct Outcome {
    pub(super) name: &'static str,
    pub(super) reply: Json,
    pub(super) chunks: Json,
    pub(super) error: Json,
}

pub(super) fn calls(values: Vec<Json>) -> Json {
    let mut fields = Extra::new();
    fields.insert("calls".to_owned(), Json::List(values));
    Json::Object(fields)
}

pub(super) fn results(values: Vec<Json>) -> Json {
    let mut fields = Extra::new();
    fields.insert("results".to_owned(), Json::List(values));
    Json::Object(fields)
}

pub(super) fn report(source: &ScriptedSource, recorder: &Recorder, outcome: Outcome) -> Json {
    let mut fields = Extra::new();
    fields.insert("outcome".to_owned(), Json::Text(outcome.name.to_owned()));
    fields.insert("requests".to_owned(), Json::List(source.requests()));
    fields.insert("pre".to_owned(), Json::List(recorder.pre.clone()));
    fields.insert("post".to_owned(), Json::List(recorder.post.clone()));
    fields.insert("reply".to_owned(), outcome.reply);
    fields.insert("chunks".to_owned(), outcome.chunks);
    fields.insert("error".to_owned(), outcome.error);
    Json::Object(fields)
}

pub(super) fn failure(error: &TextCompletionError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "TextCompletionError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// the input, headers and target a callback saw; an async call logs no target of its own
fn call_fields(input: &Json, args: &AdditionalArgs, hidden: bool) -> Extra {
    let mut fields = Extra::new();
    fields.insert("input".to_owned(), input.clone());
    fields.insert(
        "headers".to_owned(),
        args.headers
            .as_ref()
            .map_or(Json::Null, |headers| Json::Object(headers.clone())),
    );
    let (api_base, api_version) = if hidden {
        (Json::Null, Json::Null)
    } else {
        (
            args.api_base
                .as_deref()
                .map_or(Json::Null, |text| Json::Text(text.to_owned())),
            args.passthrough
                .get("api_version")
                .cloned()
                .unwrap_or(Json::Null),
        )
    };
    fields.insert("api_base".to_owned(), api_base);
    fields.insert("api_version".to_owned(), api_version);
    fields
}

pub(super) fn pre_value(call: &PreCall) -> Json {
    let args = &call.additional_args;
    let acompletion = matches!(args.passthrough.get("acompletion"), Some(Json::Bool(true)));
    let mut fields = call_fields(&call.input, args, acompletion);
    fields.insert("acompletion".to_owned(), Json::Bool(acompletion));
    fields.insert(
        "complete_input_dict".to_owned(),
        args.complete_input_dict
            .as_ref()
            .map_or(Json::Null, Json::from),
    );
    Json::Object(fields)
}

pub(super) fn post_value(call: &PostCall) -> Json {
    let mut fields = call_fields(&call.input, &call.additional_args, false);
    let original_id = match &call.original_response {
        Json::Object(original) => original.get("id").cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    };
    fields.insert("original_id".to_owned(), original_id);
    Json::Object(fields)
}

pub(super) fn error_value(error: &AzureTextError) -> Json {
    let status = error
        .status
        .map_or(Json::Null, |value| Json::Number(Number::from(value)));
    let retry_after = error
        .headers
        .as_ref()
        .and_then(|headers| headers.0.get("retry-after"))
        .map_or(Json::Null, |value| Json::Text(value.clone()));
    let mut fields = Extra::new();
    fields.insert("status".to_owned(), status);
    fields.insert("message".to_owned(), Json::Text(error.message.clone()));
    fields.insert("retry_after".to_owned(), retry_after);
    Json::Object(fields)
}

fn object(value: &Json) -> Option<&Extra> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn choice_value(choice: &Json) -> Json {
    let Some(fields) = object(choice) else {
        return Json::Null;
    };
    let message = fields.get("message").and_then(object);
    let mut out = Extra::new();
    out.insert(
        "index".to_owned(),
        fields.get("index").cloned().unwrap_or(Json::Null),
    );
    out.insert(
        "finish_reason".to_owned(),
        fields.get("finish_reason").cloned().unwrap_or(Json::Null),
    );
    for name in ["content", "role", "tool_calls", "function_call"] {
        out.insert(
            name.to_owned(),
            message
                .and_then(|body| body.get(name))
                .cloned()
                .unwrap_or(Json::Null),
        );
    }
    Json::Object(out)
}

fn usage_value(usage: &Json) -> Json {
    let Json::Object(fields) = usage else {
        return Json::Null;
    };
    let details = fields.get("completion_tokens_details");
    let present = match details {
        None | Some(Json::Null) => false,
        Some(_details) => true,
    };
    let reasoning = match fields.get("reasoning_tokens") {
        Some(Json::Null) | None => details
            .and_then(object)
            .and_then(|fields_of_details| fields_of_details.get("reasoning_tokens"))
            .cloned()
            .unwrap_or(Json::Null),
        Some(value) => value.clone(),
    };
    let mut out = Extra::new();
    for name in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        out.insert(
            name.to_owned(),
            fields
                .get(name)
                .cloned()
                .unwrap_or(Json::Number(Number::from(0_u8))),
        );
    }
    out.insert("reasoning_tokens".to_owned(), reasoning);
    out.insert(
        "queue_time".to_owned(),
        fields.get("queue_time").cloned().unwrap_or(Json::Null),
    );
    out.insert("details".to_owned(), Json::Bool(present));
    Json::Object(out)
}

pub(super) fn reply_value(reply: &ChatReply) -> Json {
    let created = match &reply.created {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    };
    let choices: Vec<Json> = reply.choices.iter().map(choice_value).collect();
    let usage = reply.usage.as_ref().map_or(Json::Null, usage_value);
    let mut fields = Extra::new();
    fields.insert("id".to_owned(), reply.id.clone());
    fields.insert("model".to_owned(), reply.model.clone());
    fields.insert(
        "created_is_current".to_owned(),
        Json::Bool(created.is_some_and(|value| value > CREATED_FLOOR)),
    );
    fields.insert("choices".to_owned(), Json::List(choices));
    fields.insert("usage".to_owned(), usage);
    Json::Object(fields)
}

pub(super) fn chunk_value(chunk: &StreamChunk) -> Json {
    let choice = chunk.choices.first();
    let content = choice
        .and_then(|entry| entry.delta.content.clone())
        .map_or(Json::Null, Json::Text);
    let role = choice
        .and_then(|entry| entry.delta.role.clone())
        .map_or(Json::Null, Json::Text);
    let finish_reason = choice
        .and_then(|entry| entry.finish_reason.clone())
        .map_or(Json::Null, Json::Text);
    let index = choice.map_or(Json::Null, |entry| Json::Number(Number::from(entry.index)));
    let mut fields = Extra::new();
    fields.insert("content".to_owned(), content);
    fields.insert("role".to_owned(), role);
    fields.insert("finish_reason".to_owned(), finish_reason);
    fields.insert("index".to_owned(), index);
    fields.insert("model".to_owned(), Json::Text(chunk.model.clone()));
    fields.insert(
        "created_is_current".to_owned(),
        Json::Bool(chunk.created > CREATED_FLOOR),
    );
    Json::Object(fields)
}
