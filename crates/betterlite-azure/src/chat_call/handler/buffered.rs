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
use super::{records, require_kind};

const NOT_AN_OBJECT: &str = "the request body is not a JSON object";

// get_azure_openai_client for the sync path, whose failures completion raises unchanged
fn sync_client<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
) -> Result<ClientHandle<Source::Client>, AzureChatError>
where
    Source: ClientSource,
{
    let client = source
        .get(GetClient {
            api_key: call.api_key,
            api_base: Some(call.api_base),
            api_version: Some(call.api_version),
            model: Some(sent.model.as_str()),
            litellm_params: &sent.litellm_params,
            is_async: false,
            supplied: sent.supplied.as_ref(),
        })
        .map_err(|error| AzureChatError::from(&error))?;
    require_kind(client, false)
}

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
    let client = sync_client(source, sent, call)?;
    let raw = send_sync(&client, sent, call, runtime).await?;
    let headers = SecretHeaders::from_response(&raw.headers);
    if let Some(text) = raw.body.as_str() {
        return Err(AzureChatError::azure(500, string_response(text)));
    }
    let original = serde_json::from_value(raw.body).map_err(AzureChatError::from)?;
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
    let client = async_client(source, sent, call, hooks)?;
    let raw = send_async(&client, sent, call, runtime, hooks).await?;
    let headers = SecretHeaders::from_response(&raw.headers);
    hooks.response_headers(&response::raw_headers(&headers));
    if let Some(text) = raw.body.as_str() {
        return Err(logged_failure(
            sent,
            hooks,
            AzureChatError::azure(500, string_response(text)),
        ));
    }
    let original = serde_json::from_value(raw.body).map_err(AzureChatError::from)?;
    hooks.post_call(records::async_post_call(&sent.data, original.clone()));
    project(
        &original,
        &headers,
        sent.json_mode,
        true,
        runtime.identifier,
        runtime.clock,
    )
}

// every async failure in the prelude runs post_call with str(e) and comes back unchanged
fn async_client<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    hooks: &mut dyn ChatHooks,
) -> Result<ClientHandle<Source::Client>, AzureChatError>
where
    Source: ClientSource,
{
    let resolved = match source.get(GetClient {
        api_key: call.api_key,
        api_base: Some(call.api_base),
        api_version: Some(call.api_version),
        model: Some(sent.model.as_str()),
        litellm_params: &sent.litellm_params,
        is_async: true,
        supplied: sent.supplied.as_ref(),
    }) {
        Ok(client) => client,
        Err(error) => return Err(logged_failure(sent, hooks, AzureChatError::from(&error))),
    };
    let client = match require_kind(resolved, true) {
        Ok(client) => client,
        Err(failure) => return Err(logged_failure(sent, hooks, failure)),
    };
    match hooks.pre_call(records::async_pre_call(call, &client, &sent.data)) {
        Ok(()) => Ok(client),
        Err(error) => Err(logged_failure(
            sent,
            hooks,
            AzureChatError::azure(500, error.to_string()),
        )),
    }
}

async fn send_sync<Source>(
    client: &ClientHandle<Source::Client>,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
) -> Result<RawChatResponse, AzureChatError>
where
    Source: ClientSource,
{
    let value = serde_json::to_value(&sent.data).map_err(AzureChatError::from)?;
    let Some(object) = value.as_object() else {
        return Err(AzureChatError::azure(500, NOT_AN_OBJECT.to_owned()));
    };
    let recovery_args = recovery(runtime);
    send_raw_chat_sync(&client.transport, object, call.timeout, &recovery_args)
        .await
        .map_err(|error| AzureChatError::wrapped(&error))
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
    let value = serde_json::to_value(&sent.data).map_err(AzureChatError::from)?;
    let Some(object) = value.as_object() else {
        return Err(logged_failure(
            sent,
            hooks,
            AzureChatError::azure(500, NOT_AN_OBJECT.to_owned()),
        ));
    };
    let recovery_args = recovery(runtime);
    let send = send_raw_chat_async(
        &client.transport,
        object,
        call.timeout,
        runtime.now,
        &recovery_args,
    );
    let answer = match call.cancel {
        None => send.await,
        Some(token) => tokio::select! {
            biased;
            answer = send => answer,
            () = token.cancelled() => return Err(logged_failure(sent, hooks, AzureChatError::Cancelled)),
        },
    };
    answer.map_err(|error| logged_failure(sent, hooks, AzureChatError::wrapped(&error)))
}

fn recovery<'runtime>(runtime: &'runtime Runtime<'_>) -> Recovery<'runtime> {
    Recovery {
        clock: runtime.recovered_clock,
        next_id: runtime.identifier,
        tokens: runtime.prompt_tokens,
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
