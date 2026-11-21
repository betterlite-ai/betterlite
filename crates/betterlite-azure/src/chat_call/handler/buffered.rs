use betterlite_models::decisions;
use betterlite_openai::output_limits::build::Recovery;
use betterlite_openai::output_limits::raw_send::{
    RawChatResponse, send_raw_chat_async, send_raw_chat_sync,
};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;

use super::super::arguments::{ChatCall, Runtime, SentChat};
use super::super::client::{ClientHandle, ClientSource, GetClient};
use super::super::error::{AzureChatError, MAX_RETRIES_TYPE, string_response};
use super::super::hooks::ChatHooks;
use super::super::response::{self, ChatReply, project};
use super::records;
use super::require_kind;
use super::streaming::Direction;

const NOT_AN_OBJECT: &str = "the request body is not a JSON object";

// one buffered call for both directions: the sync path logs before it validates and raises failures
// unchanged, the async path resolves the client first and logs every failure with post_call
pub async fn call<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
    direction: Direction,
) -> Result<ChatReply, AzureChatError>
where
    Source: ClientSource,
{
    if matches!(direction, Direction::Sync) {
        hooks
            .pre_call(records::sync_pre_call(
                call,
                sent.messages.clone(),
                &sent.data,
            ))
            .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
        if !sent.max_retries.is_int() {
            return Err(AzureChatError::azure(422, MAX_RETRIES_TYPE.to_owned()));
        }
    }
    let client = resolve_client(source, sent, call, hooks, direction)?;
    let raw = send(&client, sent, call, runtime, hooks, direction).await?;
    let headers = SecretHeaders::from_response(&raw.headers);
    if matches!(direction, Direction::Async) {
        hooks.response_headers(&response::raw_headers(&headers));
    }
    if let Some(text) = raw.body.as_str() {
        return Err(reported(
            direction,
            sent,
            hooks,
            AzureChatError::azure(500, string_response(text)),
        ));
    }
    let original = serde_json::from_value(raw.body).map_err(AzureChatError::from)?;
    match direction {
        Direction::Sync => hooks.post_call(records::sync_post_call(
            call,
            sent.messages.clone(),
            original.clone(),
            &headers,
        )),
        Direction::Async => hooks.post_call(records::async_post_call(&sent.data, original.clone())),
    }
    project(
        &original,
        &headers,
        sent.json_mode,
        matches!(direction, Direction::Async),
        runtime.identifier,
        runtime.clock,
    )
}

// get_azure_openai_client plus the isinstance guard, with the async pre_call after the guard
fn resolve_client<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    hooks: &mut dyn ChatHooks,
    direction: Direction,
) -> Result<ClientHandle<Source::Client>, AzureChatError>
where
    Source: ClientSource,
{
    let is_async = matches!(direction, Direction::Async);
    let request = GetClient {
        api_key: call.api_key,
        api_base: Some(call.api_base),
        api_version: Some(call.api_version),
        model: Some(sent.model.as_str()),
        litellm_params: &sent.litellm_params,
        is_async,
        supplied: sent.supplied.as_ref(),
    };
    let resolved = source
        .get(request)
        .map_err(|error| reported(direction, sent, hooks, AzureChatError::from(&error)))?;
    let client = require_kind(resolved, is_async)
        .map_err(|failure| reported(direction, sent, hooks, failure))?;
    if !is_async {
        return Ok(client);
    }
    match hooks.pre_call(records::async_pre_call(call, &client, &sent.data)) {
        Ok(()) => Ok(client),
        Err(error) => Err(logged_failure(
            sent,
            hooks,
            AzureChatError::azure(500, error.to_string()),
        )),
    }
}

async fn send<Source>(
    client: &ClientHandle<Source::Client>,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
    direction: Direction,
) -> Result<RawChatResponse, AzureChatError>
where
    Source: ClientSource,
{
    let value = serde_json::to_value(&sent.data).map_err(AzureChatError::from)?;
    let Some(object) = value.as_object() else {
        return Err(reported(
            direction,
            sent,
            hooks,
            AzureChatError::azure(500, NOT_AN_OBJECT.to_owned()),
        ));
    };
    let recovery_args = recovery(runtime);
    let answer = match direction {
        Direction::Sync => {
            send_raw_chat_sync(&client.transport, object, call.timeout, &recovery_args).await
        }
        Direction::Async => {
            let pending = send_raw_chat_async(
                &client.transport,
                object,
                call.timeout,
                runtime.now,
                &recovery_args,
            );
            match call.cancel {
                None => pending.await,
                Some(token) => tokio::select! {
                    biased;
                    answer = pending => answer,
                    () = token.cancelled() => return Err(logged_failure(sent, hooks, AzureChatError::Cancelled)),
                },
            }
        }
    };
    answer.map_err(|error| reported(direction, sent, hooks, AzureChatError::wrapped(&error)))
}

fn recovery<'runtime>(runtime: &'runtime Runtime<'_>) -> Recovery<'runtime> {
    Recovery {
        clock: runtime.recovered_clock,
        next_id: runtime.identifier,
        tokens: runtime.prompt_tokens,
    }
}

fn reported<Client>(
    direction: Direction,
    sent: &SentChat<Client>,
    hooks: &mut dyn ChatHooks,
    failure: AzureChatError,
) -> AzureChatError {
    match direction {
        Direction::Sync => failure,
        Direction::Async => logged_failure(sent, hooks, failure),
    }
}

// every async except block logs str(e) before it re-raises or wraps the failure
fn logged_failure<Client>(
    sent: &SentChat<Client>,
    hooks: &mut dyn ChatHooks,
    failure: AzureChatError,
) -> AzureChatError {
    hooks.post_call(records::async_post_call(
        &sent.data,
        decisions::Json::Text(failure.text()),
    ));
    failure
}
