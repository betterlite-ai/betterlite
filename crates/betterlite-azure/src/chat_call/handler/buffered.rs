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

// the built request, the call that made it and the logging hooks every step reports to
struct Exchange<'a, Client> {
    sent: &'a SentChat<Client>,
    call: &'a ChatCall<'a, Client>,
    hooks: &'a mut dyn ChatHooks,
}

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
    let mut exchange = Exchange { sent, call, hooks };
    if !exchange.call.acompletion {
        prepare_sync(&mut exchange)?;
    }
    // get_azure_openai_client, then the isinstance guard over the client it handed back
    let request = GetClient {
        api_key: exchange.call.api_key,
        api_base: Some(exchange.call.api_base),
        api_version: Some(exchange.call.api_version),
        model: Some(exchange.sent.model.as_str()),
        litellm_params: &exchange.sent.litellm_params,
        is_async: exchange.call.acompletion,
        supplied: exchange.sent.supplied.as_ref(),
    };
    let resolved = source
        .get(request)
        .map_err(|error| logged_failure(&mut exchange, AzureChatError::from(&error)))?;
    let client = require_kind(resolved, exchange.call.acompletion)
        .map_err(|failure| logged_failure(&mut exchange, failure))?;
    if exchange.call.acompletion {
        prepare_async(&mut exchange, &client)?;
    }
    let raw = send(&client, &mut exchange, runtime).await?;
    finish(&mut exchange, runtime, raw)
}

// the sync pre call logs the request before the retry count is validated
fn prepare_sync<Client>(exchange: &mut Exchange<'_, Client>) -> Result<(), AzureChatError> {
    exchange
        .hooks
        .pre_call(records::sync_pre_call(
            exchange.call,
            exchange.sent.messages.clone(),
            &exchange.sent.data,
        ))
        .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    if exchange.sent.max_retries.is_int() {
        return Ok(());
    }
    Err(AzureChatError::azure(422, MAX_RETRIES_TYPE.to_owned()))
}

// the async pre call logs the client the send will use
fn prepare_async<Client>(
    exchange: &mut Exchange<'_, Client>,
    client: &ClientHandle<Client>,
) -> Result<(), AzureChatError> {
    exchange
        .hooks
        .pre_call(records::async_pre_call(
            exchange.call,
            client,
            &exchange.sent.data,
        ))
        .map_err(|error| logged_failure(exchange, AzureChatError::azure(500, error.to_string())))
}

async fn send<Source>(
    client: &ClientHandle<Source::Client>,
    exchange: &mut Exchange<'_, Source::Client>,
    runtime: &Runtime<'_>,
) -> Result<RawChatResponse, AzureChatError>
where
    Source: ClientSource,
{
    let value = serde_json::to_value(&exchange.sent.data).map_err(AzureChatError::from)?;
    let Some(object) = value.as_object() else {
        return Err(logged_failure(
            exchange,
            AzureChatError::azure(500, NOT_AN_OBJECT.to_owned()),
        ));
    };
    let recovery_args = recovery(runtime);
    let answer = if exchange.call.acompletion {
        let pending = send_raw_chat_async(
            &client.transport,
            object,
            exchange.call.timeout,
            runtime.now,
            &recovery_args,
        );
        match exchange.call.cancel {
            None => pending.await,
            Some(token) => tokio::select! {
                biased;
                answer = pending => answer,
                () = token.cancelled() => return Err(logged_failure(
                    exchange,
                    AzureChatError::Cancelled,
                )),
            },
        }
    } else {
        send_raw_chat_sync(
            &client.transport,
            object,
            exchange.call.timeout,
            &recovery_args,
        )
        .await
    };
    answer.map_err(|error| logged_failure(exchange, AzureChatError::wrapped(&error)))
}

// the response the SDK returned: headers, the string-reply guard and the post call, then the projection
fn finish<Client>(
    exchange: &mut Exchange<'_, Client>,
    runtime: &Runtime<'_>,
    raw: RawChatResponse,
) -> Result<ChatReply, AzureChatError> {
    let headers = SecretHeaders::from_response(&raw.headers);
    if exchange.call.acompletion {
        exchange
            .hooks
            .response_headers(&response::raw_headers(&headers));
    }
    if let Some(text) = raw.body.as_str() {
        return Err(logged_failure(
            exchange,
            AzureChatError::azure(500, string_response(text)),
        ));
    }
    let original: decisions::Json =
        serde_json::from_value(raw.body).map_err(AzureChatError::from)?;
    if exchange.call.acompletion {
        exchange.hooks.post_call(records::async_post_call(
            &exchange.sent.data,
            original.clone(),
        ));
    } else {
        exchange.hooks.post_call(records::sync_post_call(
            exchange.call,
            exchange.sent.messages.clone(),
            original.clone(),
            &headers,
        ));
    }
    let options = ReplyOptions {
        json_mode: exchange.sent.json_mode,
        provider: None,
        base_model: None,
        identifier: runtime.identifier,
        clock: runtime.clock,
    };
    project(&original, &headers, &options, exchange.call.acompletion)
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
    exchange: &mut Exchange<'_, Client>,
    failure: AzureChatError,
) -> AzureChatError {
    if exchange.call.acompletion {
        exchange.hooks.post_call(records::async_post_call(
            &exchange.sent.data,
            decisions::Json::Text(failure.text()),
        ));
    }
    failure
}
