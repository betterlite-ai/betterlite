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

// one body for both directions; only the async path stores headers and logs failures with post_call
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
    let is_async = matches!(direction, Direction::Async);
    if !is_async {
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
        .map_err(|error| logged_failure(direction, sent, hooks, AzureChatError::from(&error)))?;
    let client = require_kind(resolved, is_async)
        .map_err(|failure| logged_failure(direction, sent, hooks, failure))?;
    if is_async {
        hooks
            .pre_call(records::async_pre_call(call, &client, &sent.data))
            .map_err(|error| {
                logged_failure(
                    direction,
                    sent,
                    hooks,
                    AzureChatError::azure(500, error.to_string()),
                )
            })?;
    }
    let raw = send(&client, sent, call, runtime, hooks, direction).await?;
    let headers = SecretHeaders::from_response(&raw.headers);
    if is_async {
        hooks.response_headers(&response::raw_headers(&headers));
    }
    if let Some(text) = raw.body.as_str() {
        return Err(logged_failure(
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
        is_async,
        runtime.identifier,
        runtime.clock,
    )
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
        return Err(logged_failure(
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
                    () = token.cancelled() => return Err(logged_failure(direction, sent, hooks, AzureChatError::Cancelled)),
                },
            }
        }
    };
    answer.map_err(|error| logged_failure(direction, sent, hooks, AzureChatError::wrapped(&error)))
}

fn recovery<'runtime>(runtime: &'runtime Runtime<'_>) -> Recovery<'runtime> {
    Recovery {
        clock: runtime.recovered_clock,
        next_id: runtime.identifier,
        tokens: runtime.prompt_tokens,
    }
}

// the async except blocks log str(e) before they re-raise or wrap the failure; the sync path raises it
fn logged_failure<Client>(
    direction: Direction,
    sent: &SentChat<Client>,
    hooks: &mut dyn ChatHooks,
    failure: AzureChatError,
) -> AzureChatError {
    if matches!(direction, Direction::Async) {
        hooks.post_call(records::async_post_call(
            &sent.data,
            decisions::Json::Text(failure.text()),
        ));
    }
    failure
}
