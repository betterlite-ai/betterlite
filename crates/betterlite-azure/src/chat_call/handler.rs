use crate::azure_stream_errors::stream::AzureChatStream;
use crate::endpoints::initialize::EndpointError;

use super::arguments::{ChatCall, Runtime, SentChat};
use super::client::{
    ClientHandle, ClientSource, GatewayClient, GetClient, is_async_chat, is_sync_chat,
};
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
    let json_mode = request::pop_json_mode(call.optional_params);
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
    if call.acompletion && request::wants_async_stream(call.optional_params) {
        let stream = streaming::async_streaming(source, sent, call, runtime, hooks).await?;
        return Ok(ChatOutcome::Stream(stream));
    }
    if call.acompletion {
        let reply = buffered::acompletion(source, sent, call, runtime, hooks)
            .await
            .map_err(AzureChatError::wrap_source)?;
        return Ok(ChatOutcome::Buffered(Box::new(reply)));
    }
    if request::wants_sync_stream(call.optional_params) {
        let stream = streaming::streaming(source, sent, call, runtime, hooks).await?;
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
    let (true, None) = (cloudflare, call.supplied.as_ref()) else {
        return Ok(call.supplied.clone());
    };
    source
        .gateway(GatewayClient {
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
        })
        .map(Some)
        .map_err(|error| AzureChatError::from_endpoint(&error))
}

// get_azure_openai_client(client=...): a supplied client comes back as it is
fn resolve_client<Source>(
    source: &Source,
    sent: &SentChat<Source::Client>,
    call: &ChatCall<'_, Source::Client>,
    is_async: bool,
) -> Result<ClientHandle<Source::Client>, EndpointError>
where
    Source: ClientSource,
{
    source.get(GetClient {
        api_key: call.api_key,
        api_base: Some(call.api_base),
        api_version: Some(call.api_version),
        model: Some(sent.model.as_str()),
        litellm_params: &sent.litellm_params,
        is_async,
        supplied: sent.supplied.as_ref(),
    })
}

// isinstance guards: the sync raise is an AzureOpenAIError, the async ValueError becomes one
fn require_sync<Client>(
    client: ClientHandle<Client>,
) -> Result<ClientHandle<Client>, AzureChatError> {
    if is_sync_chat(client.kind) {
        Ok(client)
    } else {
        Err(AzureChatError::azure(500, SYNC_CLIENT.to_owned()))
    }
}

fn require_async<Client>(
    client: ClientHandle<Client>,
) -> Result<ClientHandle<Client>, AzureChatError> {
    if is_async_chat(client.kind) {
        Ok(client)
    } else {
        Err(AzureChatError::azure(500, ASYNC_CLIENT.to_owned()))
    }
}
