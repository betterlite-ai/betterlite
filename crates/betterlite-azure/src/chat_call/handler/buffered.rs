use betterlite_models::decisions;
use betterlite_openai::compatible_wire::ReplyOptions;
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

const NOT_AN_OBJECT: &str = "the request body is not a JSON object";

// one body for both directions; only the async path stores headers and logs failures with post_call
pub async fn call<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
) -> Result<ChatReply, AzureChatError>
where
    Source: ClientSource,
{
    if !call.acompletion {
        prepare_sync(sent, call, hooks)?;
    }
    let client = resolve(source, sent, call, hooks)?;
    if call.acompletion {
        prepare_async(sent, call, &client, hooks)?;
    }
    let raw = send(&client, sent, call, runtime, hooks).await?;
    finish(sent, call, runtime, hooks, raw)
}

// the sync pre call logs the request before the retry count is validated
fn prepare_sync<Client>(
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
        return Ok(());
    }
    Err(AzureChatError::azure(422, MAX_RETRIES_TYPE.to_owned()))
}

// the async pre call logs the client the send will use
fn prepare_async<Client>(
    sent: &SentChat<Client>,
    call: &ChatCall<'_, Client>,
    client: &ClientHandle<Client>,
    hooks: &mut dyn ChatHooks,
) -> Result<(), AzureChatError> {
    hooks
        .pre_call(records::async_pre_call(call, client, &sent.data))
        .map_err(|error| {
            logged_failure(
                call,
                sent,
                hooks,
                AzureChatError::azure(500, error.to_string()),
            )
        })
}

// get_azure_openai_client, then the isinstance guard over the client it handed back
fn resolve<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    hooks: &mut dyn ChatHooks,
) -> Result<ClientHandle<Source::Client>, AzureChatError>
where
    Source: ClientSource,
{
    let request = GetClient {
        api_key: call.api_key,
        api_base: Some(call.api_base),
        api_version: Some(call.api_version),
        model: Some(sent.model.as_str()),
        litellm_params: &sent.litellm_params,
        is_async: call.acompletion,
        supplied: sent.supplied.as_ref(),
    };
    let resolved = source
        .get(request)
        .map_err(|error| logged_failure(call, sent, hooks, AzureChatError::from(&error)))?;
    require_kind(resolved, call.acompletion)
        .map_err(|failure| logged_failure(call, sent, hooks, failure))
}

async fn send<Source>(
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
            call,
            sent,
            hooks,
            AzureChatError::azure(500, NOT_AN_OBJECT.to_owned()),
        ));
    };
    let recovery_args = recovery(runtime);
    let answer = if call.acompletion {
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
                () = token.cancelled() => return Err(logged_failure(
                    call,
                    sent,
                    hooks,
                    AzureChatError::Cancelled,
                )),
            },
        }
    } else {
        send_raw_chat_sync(&client.transport, object, call.timeout, &recovery_args).await
    };
    answer.map_err(|error| logged_failure(call, sent, hooks, AzureChatError::wrapped(&error)))
}

// the response the SDK returned: headers, the string-reply guard and the post call, then the projection
fn finish<Client>(
    sent: &SentChat<Client>,
    call: &ChatCall<'_, Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
    raw: RawChatResponse,
) -> Result<ChatReply, AzureChatError> {
    let headers = SecretHeaders::from_response(&raw.headers);
    if call.acompletion {
        hooks.response_headers(&response::raw_headers(&headers));
    }
    if let Some(text) = raw.body.as_str() {
        return Err(logged_failure(
            call,
            sent,
            hooks,
            AzureChatError::azure(500, string_response(text)),
        ));
    }
    let original: decisions::Json =
        serde_json::from_value(raw.body).map_err(AzureChatError::from)?;
    if call.acompletion {
        hooks.post_call(records::async_post_call(&sent.data, original.clone()));
    } else {
        hooks.post_call(records::sync_post_call(
            call,
            sent.messages.clone(),
            original.clone(),
            &headers,
        ));
    }
    let options = ReplyOptions {
        json_mode: sent.json_mode,
        provider: None,
        base_model: None,
        identifier: runtime.identifier,
        clock: runtime.clock,
    };
    project(&original, &headers, &options, call.acompletion)
}

const fn recovery<'runtime>(runtime: &'runtime Runtime<'_>) -> Recovery<'runtime> {
    Recovery {
        clock: runtime.recovered_clock,
        next_id: runtime.identifier,
        tokens: runtime.prompt_tokens,
    }
}

// the async except blocks log str(e) before they re-raise or wrap the failure; the sync path raises it
fn logged_failure<Client>(
    call: &ChatCall<'_, Client>,
    sent: &SentChat<Client>,
    hooks: &mut dyn ChatHooks,
    failure: AzureChatError,
) -> AzureChatError {
    if call.acompletion {
        hooks.post_call(records::async_post_call(
            &sent.data,
            decisions::Json::Text(failure.text()),
        ));
    }
    failure
}
