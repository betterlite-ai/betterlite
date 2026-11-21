use betterlite_azure::azure_stream_errors::records::PreCallRecord;
use betterlite_azure::chat_call::error::AzureChatError;
use betterlite_azure::chat_call::response::ChatReply;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value, json};

use super::hooks::Recorder;
use super::source::ScriptedSource;

const TIMEOUT_DIAGNOSTIC: &str = "- timeout value=";

pub(super) struct Outcome {
    pub(super) name: &'static str,
    pub(super) reply: Value,
    pub(super) error: Value,
    pub(super) forward: Value,
}

pub(super) fn calls(values: Vec<Value>) -> Value {
    let mut fields = Map::new();
    fields.insert("calls".to_owned(), Value::Array(values));
    Value::Object(fields)
}

pub(super) fn report(source: &ScriptedSource, recorder: &Recorder, outcome: &Outcome) -> Value {
    json!({
        "outcome": outcome.name,
        "gets": gets_value(source),
        "gateways": gateways_value(source),
        "requests": requests_value(source),
        "pre": pre_values(recorder),
        "post": post_values(recorder),
        "response_headers": response_headers_value(recorder),
        "reply": outcome.reply,
        "error": outcome.error,
        "forward": outcome.forward,
    })
}

fn response_headers_value(recorder: &Recorder) -> Value {
    recorder
        .response_headers
        .last()
        .map_or(Value::Null, |headers| {
            Value::Object(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Value::String(value.clone())))
                    .collect(),
            )
        })
}

fn gets_value(source: &ScriptedSource) -> Value {
    Value::Array(
        source
            .gets()
            .iter()
            .map(|record| {
                json!({
                    "api_version": record.api_version,
                    "api_base": record.api_base,
                    "api_key": record.api_key,
                    "model": record.model,
                    "is_async": record.is_async,
                    "supplied": record.supplied,
                    "litellm_params": {
                        "azure_ad_token": record.azure_ad_token,
                        "max_retries": record.max_retries,
                        "timeout": record.timeout,
                    },
                })
            })
            .collect(),
    )
}

fn gateways_value(source: &ScriptedSource) -> Value {
    Value::Array(
        source
            .gateways()
            .iter()
            .map(|record| {
                json!({
                    "api_base": record.api_base,
                    "model": record.model,
                    "api_version": record.api_version,
                    "max_retries": record.max_retries,
                    "timeout": record.timeout,
                    "api_key": record.api_key,
                    "acompletion": record.acompletion,
                })
            })
            .collect(),
    )
}

fn requests_value(source: &ScriptedSource) -> Value {
    Value::Array(
        source
            .requests()
            .iter()
            .map(|record| {
                json!({
                    "data": record.data,
                    "caller_headers": record.caller_headers,
                })
            })
            .collect(),
    )
}

fn pre_values(recorder: &Recorder) -> Value {
    Value::Array(recorder.pre.iter().map(pre_value).collect())
}

fn post_values(recorder: &Recorder) -> Value {
    Value::Array(recorder.post.iter().map(post_value).collect())
}

fn headers_value(args: &AdditionalArgs) -> Value {
    args.headers.as_ref().map_or(Value::Null, |headers| {
        Value::Object(
            headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        serde_json::to_value(value).unwrap_or(Value::Null),
                    )
                })
                .collect(),
        )
    })
}

fn args_fields(args: &AdditionalArgs) -> Map<String, Value> {
    let acompletion = matches!(
        args.passthrough.get("acompletion"),
        Some(DecisionsJson::Bool(true))
    );
    let mut fields = Map::new();
    fields.insert("headers".to_owned(), headers_value(args));
    fields.insert(
        "api_base".to_owned(),
        args.api_base
            .as_ref()
            .map_or(Value::Null, |text| Value::String(text.clone())),
    );
    fields.insert(
        "api_version".to_owned(),
        serde_json::to_value(args.passthrough.get("api_version")).unwrap_or(Value::Null),
    );
    fields.insert("acompletion".to_owned(), Value::Bool(acompletion));
    fields.insert(
        "openai_sdk".to_owned(),
        args.openai_sdk.map_or(Value::Null, Value::Bool),
    );
    let body = args
        .complete_input_dict
        .as_ref()
        .map_or(Value::Null, |raw| {
            serde_json::to_value(DecisionsJson::from(raw)).unwrap_or(Value::Null)
        });
    fields.insert("complete_input_dict".to_owned(), body);
    fields
}

fn pre_value(record: &PreCallRecord) -> Value {
    let mut fields = args_fields(&record.additional_args);
    fields.insert(
        "input".to_owned(),
        serde_json::to_value(&record.input).unwrap_or(Value::Null),
    );
    fields.insert(
        "api_key".to_owned(),
        record.api_key.as_ref().map_or(Value::Null, |key| {
            Value::String(key.expose_secret().to_owned())
        }),
    );
    Value::Object(fields)
}

fn post_value(call: &PostCall) -> Value {
    let mut fields = args_fields(&call.additional_args);
    fields.insert(
        "input".to_owned(),
        serde_json::to_value(&call.input).unwrap_or(Value::Null),
    );
    let original_id = match &call.original_response {
        DecisionsJson::Object(original) => original.get("id").cloned().map_or(Value::Null, |id| {
            serde_json::to_value(id).unwrap_or(Value::Null)
        }),
        DecisionsJson::Null
        | DecisionsJson::Bool(_)
        | DecisionsJson::Number(_)
        | DecisionsJson::Text(_)
        | DecisionsJson::List(_) => Value::Null,
    };
    fields.insert("original_id".to_owned(), original_id);
    Value::Object(fields)
}

pub(super) fn reply_value(reply: &ChatReply) -> Value {
    let response = &reply.response;
    let choice = response.choices.first().map_or(Value::Null, |choice| {
        json!({
            "index": choice.index,
            "finish_reason": choice.finish_reason,
            "role": choice.message.role,
            "content": choice.message.content,
            "has_tool_calls": choice.message.tool_calls.is_some(),
        })
    });
    json!({
        "id": response.id,
        "model": response.model,
        "created": response.created,
        "choice": choice,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
        "hidden": serde_json::to_value(&reply.hidden_params).unwrap_or(Value::Null),
    })
}

pub(super) fn error_value(error: &AzureChatError) -> Value {
    match error {
        AzureChatError::Azure {
            status,
            message,
            headers,
            body,
        } => error_fields(Some(*status), message, headers.as_ref(), body.as_deref()),
        AzureChatError::Cancelled => Value::Null,
    }
}

fn error_fields(
    status: Option<u16>,
    message: &str,
    headers: Option<&SecretHeaders>,
    body: Option<&DecisionsJson>,
) -> Value {
    json!({
        "status": status,
        "retry_after": headers
            .and_then(|values| values.0.get("retry-after"))
            .cloned()
            .map_or(Value::Null, Value::String),
        "has_headers": headers.is_some(),
        "has_body": body.is_some(),
        "has_timeout_diagnostic": message.contains(TIMEOUT_DIAGNOSTIC),
    })
}
