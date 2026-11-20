use betterlite_models::decisions;
use betterlite_openai::output_limits::build::Recovery;
use betterlite_openai::output_limits::raw_send::{
    RawChatResponse, send_raw_chat_async, send_raw_chat_sync,
};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;

use super::super::arguments::{ChatCall, Runtime, SentChat};
use super::super::client::{ClientHandle, ClientSource};
use super::super::error::{AzureChatError, MAX_RETRIES_TYPE, string_response};
use super::super::hooks::ChatHooks;
use super::super::response::{self, ChatReply, project};
use super::{records, require_async, require_sync, resolve_client};

const NOT_AN_OBJECT: &str = "the request body is not a JSON object";

// the sync buffered call logs before it validates the retry count, then converts the parsed reply
pub async fn complete_buffered<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
) -> Result<ChatReply, AzureChatError>
where
    Source: ClientSource,
{
    sync_prelude(sent, call, hooks)?;
    let resolved = resolve_client(source, sent, call, false)
        .map_err(|error| AzureChatError::from_endpoint(&error))?;
    let client = require_sync(resolved)?;
    let value = serde_json::to_value(&sent.data)
        .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    let Some(object) = value.as_object() else {
        return Err(AzureChatError::azure(500, NOT_AN_OBJECT.to_owned()));
    };
    let raw = send_raw_chat_sync(&client.transport, object, call.timeout, &recovery(runtime))
        .await
        .map_err(|error| AzureChatError::wrapped(&error))?;
    let (headers, original) = parse_reply(raw)?;
    hooks.post_call(records::sync_post_call(
        call,
        sent.messages.clone(),
        original.clone(),
        &headers,
    ));
    project(
        &original,
        &headers,
        sent.json_mode,
        false,
        runtime.identifier,
        runtime.clock,
    )
}

// acompletion converts the SDK model dump and stores the response headers for logging
pub async fn acompletion<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
) -> Result<ChatReply, AzureChatError>
where
    Source: ClientSource,
{
    let client = async_prelude(source, sent, call, hooks)?;
    let raw = send_async(&client, sent, call, runtime, hooks).await?;
    let headers = SecretHeaders::from_response(&raw.headers);
    hooks.response_headers(&response::raw_headers(&headers));
    let original = async_reply(raw, sent, call, hooks)?;
    hooks.post_call(records::async_post_call(call, &sent.data, original.clone()));
    project(
        &original,
        &headers,
        sent.json_mode,
        true,
        runtime.identifier,
        runtime.clock,
    )
}

fn sync_prelude<Client>(
    sent: &SentChat<Client>,
    call: &ChatCall<'_, Client>,
    hooks: &mut dyn ChatHooks,
) -> Result<(), AzureChatError> {
    hooks
        .pre_call(records::sync_pre_call(
            call,
            sent.messages.clone(),
            &sent.data,
        ))
        .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    if sent.max_retries.is_int() {
        Ok(())
    } else {
        Err(AzureChatError::azure(422, MAX_RETRIES_TYPE.to_owned()))
    }
}

fn parse_reply(raw: RawChatResponse) -> Result<(SecretHeaders, decisions::Json), AzureChatError> {
    let headers = SecretHeaders::from_response(&raw.headers);
    if let Some(text) = raw.body.as_str() {
        return Err(AzureChatError::azure(500, string_response(text)));
    }
    let original = serde_json::from_value(raw.body)
        .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    Ok((headers, original))
}

fn async_prelude<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    hooks: &mut dyn ChatHooks,
) -> Result<ClientHandle<Source::Client>, AzureChatError>
where
    Source: ClientSource,
{
    let resolved = resolve_client(source, sent, call, true).map_err(|error| {
        logged_failure(call, sent, hooks, AzureChatError::from_endpoint(&error))
    })?;
    let client =
        require_async(resolved).map_err(|failure| logged_failure(call, sent, hooks, failure))?;
    hooks
        .pre_call(records::async_pre_call(call, &client, &sent.data))
        .map_err(|error| {
            logged_failure(
                call,
                sent,
                hooks,
                AzureChatError::azure(500, error.to_string()),
            )
        })?;
    Ok(client)
}

async fn send_async<Source>(
    client: &ClientHandle<Source::Client>,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
) -> Result<RawChatResponse, AzureChatError>
where
    Source: ClientSource,
{
    let value = serde_json::to_value(&sent.data)
        .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    let Some(object) = value.as_object() else {
        return Err(AzureChatError::azure(500, NOT_AN_OBJECT.to_owned()));
    };
    let recovery = recovery(runtime);
    let send = send_raw_chat_async(
        &client.transport,
        object,
        call.timeout,
        runtime.now,
        &recovery,
    );
    let answer = match call.cancel {
        None => send.await,
        Some(token) => tokio::select! {
            biased;
            answer = send => answer,
            () = token.cancelled() => return Err(cancel_post(sent, call, hooks)),
        },
    };
    answer.map_err(|error| logged_failure(call, sent, hooks, AzureChatError::async_failure(&error)))
}

fn async_reply<Client>(
    raw: RawChatResponse,
    sent: &SentChat<Client>,
    call: &ChatCall<'_, Client>,
    hooks: &mut dyn ChatHooks,
) -> Result<decisions::Json, AzureChatError> {
    if let Some(text) = raw.body.as_str() {
        let failure = AzureChatError::azure(500, string_response(text));
        hooks.post_call(records::async_post_call(
            call,
            &sent.data,
            decisions::Json::Text(failure.text()),
        ));
        return Err(failure);
    }
    serde_json::from_value(raw.body).map_err(|error| AzureChatError::azure(500, error.to_string()))
}

fn recovery<'runtime>(runtime: &'runtime Runtime<'_>) -> Recovery<'runtime> {
    Recovery {
        clock: runtime.recovered_clock,
        next_id: runtime.identifier,
        tokens: runtime.prompt_tokens,
    }
}

// a cancelled acompletion still runs post_call, then raises Cancelled
fn cancel_post<Client>(
    sent: &SentChat<Client>,
    call: &ChatCall<'_, Client>,
    hooks: &mut dyn ChatHooks,
) -> AzureChatError {
    hooks.post_call(records::async_post_call(
        call,
        &sent.data,
        decisions::Json::Text(String::new()),
    ));
    AzureChatError::Cancelled
}

// every async except block logs str(e) before it re-raises or wraps the failure
fn logged_failure<Client>(
    call: &ChatCall<'_, Client>,
    sent: &SentChat<Client>,
    hooks: &mut dyn ChatHooks,
    failure: AzureChatError,
) -> AzureChatError {
    hooks.post_call(records::async_post_call(
        call,
        &sent.data,
        decisions::Json::Text(failure.text()),
    ));
    failure
}
