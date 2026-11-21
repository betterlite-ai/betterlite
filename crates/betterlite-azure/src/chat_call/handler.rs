use betterlite_prompts::arguments::json::Json;

use crate::azure_stream_errors::stream::AzureChatStream;
use crate::endpoints::projection::ClientKind;

use super::arguments::{ChatCall, Runtime, SentChat};
use super::client::{ClientHandle, ClientSource, GatewayClient};
use super::error::{ASYNC_CLIENT, AzureChatError, MISSING_MODEL_OR_MESSAGES, SYNC_CLIENT};
use super::hooks::ChatHooks;
use super::records;
use super::request::{self, Retries};
use super::response::ChatReply;

pub mod buffered;
pub mod streaming;

pub enum ChatOutcome {
    Buffered(Box<ChatReply>),
    Stream(AzureChatStream),
}

// AzureChatCompletion.completion: the caller's optional_params dict is consumed here
pub async fn completion<Source>(
    source: &Source,
    call: &mut ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
) -> Result<ChatOutcome, AzureChatError>
where
    Source: ClientSource,
{
    if let Some(headers) = call.headers {
        request::inject_headers(call.optional_params, headers);
    }
    let (Some(model), Some(messages)) = (call.model, call.messages) else {
        return Err(AzureChatError::azure(
            422,
            MISSING_MODEL_OR_MESSAGES.to_owned(),
        ));
    };
    let max_retries = Retries::pop(call.optional_params, call.azure.default_max_retries);
    let json_mode = matches!(
        call.optional_params.shift_remove(request::JSON_MODE_KEY),
        Some(Json::Bool(true))
    );
    let cloudflare = call.api_base.contains(request::CLOUDFLARE_MARKER);
    let supplied = supplied_client(source, call, model, &max_retries, cloudflare)?;
    let data = request::build_data(
        model,
        messages,
        call.optional_params,
        call.base_model,
        cloudflare,
        call.chat.openai_system_messages_first,
    )?;
    let sent = SentChat {
        model: model.to_owned(),
        messages: records::logged(&request::message_list(messages)?),
        data,
        max_retries,
        json_mode,
        supplied,
        litellm_params: call.litellm_params.clone(),
    };
    dispatch(source, &sent, call, runtime, hooks).await
}

// async streams on a truthy stream value, the sync branch only on a literal true
async fn dispatch<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn ChatHooks,
) -> Result<ChatOutcome, AzureChatError>
where
    Source: ClientSource,
{
    let truthy_stream = call
        .optional_params
        .get(request::STREAM_KEY)
        .is_some_and(Json::is_truthy);
    if call.acompletion && truthy_stream {
        let stream = streaming::stream(
            source,
            sent,
            call,
            runtime,
            hooks,
            streaming::Direction::Async,
        )
        .await?;
        return Ok(ChatOutcome::Stream(stream));
    }
    if call.acompletion {
        let reply = buffered::acompletion(source, sent, call, runtime, hooks).await?;
        return Ok(ChatOutcome::Buffered(Box::new(reply)));
    }
    if matches!(
        call.optional_params.get(request::STREAM_KEY),
        Some(Json::Bool(true))
    ) {
        let stream = streaming::stream(
            source,
            sent,
            call,
            runtime,
            hooks,
            streaming::Direction::Sync,
        )
        .await?;
        return Ok(ChatOutcome::Stream(stream));
    }
    let reply = buffered::complete_buffered(source, sent, call, runtime, hooks).await?;
    Ok(ChatOutcome::Buffered(Box::new(reply)))
}

// the cloudflare branch swaps the caller's client for one whose base url carries the deployment
fn supplied_client<Source>(
    source: &Source,
    call: &ChatCall<'_, Source::Client>,
    model: &str,
    retries: &Retries,
    cloudflare: bool,
) -> Result<Option<ClientHandle<Source::Client>>, AzureChatError>
where
    Source: ClientSource,
{
    if !cloudflare || call.supplied.is_some() {
        return Ok(call.supplied.clone());
    }
    let gateway = source.gateway(GatewayClient {
        api_base: call.api_base,
        model,
        api_version: Some(call.api_version),
        max_retries: retries.clone(),
        timeout: call.timeout,
        litellm_params: &call.litellm_params,
        cloudflare_params: &call.cloudflare_params,
        api_key: call.api_key,
        azure_ad_token: call.azure_ad_token,
        azure_ad_token_provider: call.azure_ad_token_provider,
        acompletion: call.acompletion,
    });
    gateway
        .map(Some)
        .map_err(|error| AzureChatError::from(&error))
}

// isinstance guards: the sync raise is an AzureOpenAIError, the async ValueError becomes one
fn require_kind<Client>(
    client: ClientHandle<Client>,
    is_async: bool,
) -> Result<ClientHandle<Client>, AzureChatError> {
    let expected = match client.kind {
        ClientKind::AzureOpenAi | ClientKind::OpenAi => !is_async,
        ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi => is_async,
        ClientKind::Other => false,
    };
    if expected {
        Ok(client)
    } else {
        let message = if is_async { ASYNC_CLIENT } else { SYNC_CLIENT };
        Err(AzureChatError::azure(500, message.to_owned()))
    }
}
