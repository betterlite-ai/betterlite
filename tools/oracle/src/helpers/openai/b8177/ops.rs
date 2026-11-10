use betterlite_openai::gigachat_chat::config::GigachatChatConfig;
use betterlite_openai::gigachat_chat::options::map_openai_params;
use betterlite_openai::gigachat_chat::reply::{ReplyInput, transform_response};
use betterlite_openai::gigachat_chat::request::{CompletionInput, completion_request};
use betterlite_openai::gigachat_stream::{StreamEvent, chunk_parser};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::input::{MapParamsStep, ReplyStep, RequestStep, Step, StreamStep};
use super::support::{FixedIds, TokenBearer, Uploader, chat_reply_value, stream_chunk_value};

pub(crate) fn execute(step: &Step, runtime: &Runtime) -> Value {
    match step {
        Step::MapParams(params) => map_params_value(params),
        Step::Request(request) => request_value(runtime, request),
        Step::Reply(reply) => reply_value(reply),
        Step::Stream(stream) => stream_value(stream),
    }
}

fn map_params_value(step: &MapParamsStep) -> Value {
    match map_openai_params(&step.params, step.optional_params.clone()) {
        Ok(params) => json!({"ok": true, "params": params}),
        Err(error) => json!({"ok": false, "status_code": error.status_code}),
    }
}

fn request_value(runtime: &Runtime, step: &RequestStep) -> Value {
    let bearer = TokenBearer {
        token: SecretString::from(step.token.as_str()),
    };
    let uploader = Uploader {
        ids: step.uploads.clone(),
    };
    let ids = FixedIds;
    let config = GigachatChatConfig {
        bearer: &bearer,
        uploader: &uploader,
        ids: &ids,
    };
    let input = CompletionInput {
        model: &step.model,
        messages: &step.messages,
        api_base: step.api_base.as_deref(),
        env_base: step.env_base.as_deref(),
        api_key: None,
        credentials: None,
        fallback_key: None,
        caller_headers: &step.headers,
        optional_params: &step.params,
    };
    match runtime.block_on(completion_request(&config, &input)) {
        Ok(request) => json!({
            "ok": true,
            "url": request.url,
            "headers": request.headers,
            "body": request.body,
        }),
        Err(error) => json!({"ok": false, "status_code": error.status_code}),
    }
}

fn reply_value(step: &ReplyStep) -> Value {
    let input = ReplyInput {
        status_code: step.status,
        body: step.body.as_bytes(),
        model: &step.model,
        created: step.created,
        structured_output: step.structured_output,
    };
    match transform_response(&FixedIds, &input) {
        Ok(response) => chat_reply_value(&response),
        Err(error) => json!({"ok": false, "status_code": error.status_code}),
    }
}

fn stream_value(step: &StreamStep) -> Value {
    let ids = FixedIds;
    let mut chunks = Vec::with_capacity(step.events.len());
    for event in &step.events {
        match chunk_parser(event.as_bytes(), &ids) {
            Ok(StreamEvent::Chunk(chunk)) => chunks.push(stream_chunk_value(&chunk)),
            Ok(StreamEvent::Done) => break,
            Err(_error) => return json!({"ok": false, "status_code": null}),
        }
    }
    json!({"ok": true, "chunks": chunks})
}
