use betterlite_bedrock::invoke_events::fake::{FakeResponse, FakeStream};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_openai::compatible_line_stream::Mode;
use betterlite_openai::compatible_wire::error::OpenAiLikeError;
use betterlite_openai::compatible_wire::{CompatibleReply, LogInput, transform_response};
use futures_util::StreamExt as _;
use indexmap::IndexMap;

use super::error::CompatibleChatError;
use super::plan::ChatPlan;
use super::stream;

const SYNC_STREAM_LOGGED: &str = "first stream response received";
// the async call logs the iterator object itself, which has no JSON form
const ASYNC_STREAM_LOGGED: &str = "stream";

pub enum ChatAnswer {
    Buffered { status: u16, body: Vec<u8> },
    Stream { body: stream::Bytes, limit: usize },
}

pub enum CompatibleChatOutcome {
    Reply(Box<CompatibleReply>),
    Stream(Box<stream::CompatibleChatStream>),
}

// e.response.text decodes a UTF-8 answer as it stands; a body that is not UTF-8 is its own error
fn status_error(status: u16, body: Vec<u8>) -> CompatibleChatError {
    match String::from_utf8(body) {
        Ok(message) => CompatibleChatError::OpenAiLike(OpenAiLikeError {
            status_code: status,
            message,
        }),
        Err(source) => CompatibleChatError::StatusBody { status, source },
    }
}

async fn sync_stream_failure(response: StreamResponse, limit: usize) -> CompatibleChatError {
    match response.buffered(limit).await {
        Ok(failed) => status_error(failed.status, failed.body),
        Err(error) => CompatibleChatError::Transport(error),
    }
}

pub async fn send_chat(
    client: &ProxiedClient,
    plan: &ChatPlan<'_>,
) -> Result<ChatAnswer, CompatibleChatError> {
    let limit = client.limits.body_bytes;
    let response = client
        .send_following(plan.request()?)
        .await
        .map_err(|error| plan.send_failure(error))?;
    let status = response.response.status().as_u16();
    if plan.buffered() {
        let answer = response
            .buffered(limit)
            .await
            .map_err(|error| plan.send_failure(error))?;
        return Ok(ChatAnswer::Buffered {
            status: answer.status,
            body: answer.body,
        });
    }
    if plan.mode == Mode::Sync && status != 200 {
        return Err(sync_stream_failure(response, limit).await);
    }
    Ok(ChatAnswer::Stream {
        body: response.body_stream(),
        limit,
    })
}

fn log_stream_post(
    plan: &ChatPlan<'_>,
    original_response: &str,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) {
    let call = PostCall {
        original_response: Json::Text(original_response.to_owned()),
        input: plan.messages.clone(),
        additional_args: AdditionalArgs {
            api_base: None,
            headers: None,
            complete_input_dict: Some(RawBody::Text(python_json::dumps_object(&plan.body))),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    post_call(state, &call, env);
}

fn selected_stream(
    plan: &ChatPlan<'_>,
    body: stream::Bytes,
    limit: usize,
) -> stream::CompatibleChatStream {
    if let Some(decoder) = plan.decoder {
        return stream::CompatibleChatStream::Decoder(decoder.decode(stream::rechunk(body)));
    }
    stream::CompatibleChatStream::Lines(stream::lines(body, limit, plan.mode))
}

fn finish_fake(body: Vec<u8>) -> Result<stream::CompatibleChatStream, CompatibleChatError> {
    let response: FakeResponse =
        serde_json::from_slice(&body).map_err(CompatibleChatError::Encode)?;
    let fake = FakeStream {
        response,
        json_mode: None,
        is_done: false,
    };
    Ok(stream::CompatibleChatStream::Fake(fake.boxed()))
}

fn finish_buffered(
    plan: &ChatPlan<'_>,
    status: u16,
    body: Vec<u8>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<CompatibleChatOutcome, CompatibleChatError> {
    if plan.stream {
        if status != 200 {
            return Err(status_error(status, body));
        }
        // make_sync_call tests the decoder first, so a supplied one reads the buffered body in 1024-byte pieces
        let selected = match plan.decoder {
            Some(decoder) => {
                stream::CompatibleChatStream::Decoder(decoder.decode(stream::rechunk(
                    futures_util::stream::iter([Ok::<Vec<u8>, HttpError>(body)]).boxed(),
                )))
            }
            None => finish_fake(body)?,
        };
        log_stream_post(plan, SYNC_STREAM_LOGGED, state, env);
        return Ok(CompatibleChatOutcome::Stream(Box::new(selected)));
    }
    if !(200..300).contains(&status) {
        return Err(status_error(status, body));
    }
    let reply = transform_response(
        &body,
        &plan.reply_options(),
        state,
        env,
        LogInput {
            messages: plan.messages.clone(),
            data: RawBody::Object(plan.body.clone()),
        },
    )
    .map_err(CompatibleChatError::Wire)?;
    Ok(CompatibleChatOutcome::Reply(Box::new(reply)))
}

pub fn finish_chat(
    plan: &ChatPlan<'_>,
    answer: ChatAnswer,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<CompatibleChatOutcome, CompatibleChatError> {
    match answer {
        ChatAnswer::Buffered { status, body } => finish_buffered(plan, status, body, state, env),
        ChatAnswer::Stream { body, limit } => {
            let selected = selected_stream(plan, body, limit);
            let logged = match plan.mode {
                Mode::Sync => SYNC_STREAM_LOGGED,
                Mode::Async => ASYNC_STREAM_LOGGED,
            };
            log_stream_post(plan, logged, state, env);
            Ok(CompatibleChatOutcome::Stream(Box::new(selected)))
        }
    }
}
