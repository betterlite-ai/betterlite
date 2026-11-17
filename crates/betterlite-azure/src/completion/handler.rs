use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::stream::CompletionStream;

use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{ClientKind, LiteLlmParams};

use super::arguments::{CompletionCall, Runtime, SentCall};
use super::client::{ClientHandle, ClientSource, GatewayClient, GetClient};
use super::error::{AzureTextError, MISSING_MODEL_OR_MESSAGES};
use super::logging::CompletionHooks;
use super::request::{self, Retries};
use super::response::ChatReply;

pub mod buffered;
mod records;
pub mod streaming;

pub enum TextCompletionOutcome {
    Buffered(Box<ChatReply>),
    Stream(CompletionStream),
}

// AzureTextCompletion.completion: the prompt comes from prompt_factory on every call
pub async fn completion<Source>(
    source: &Source,
    call: &mut CompletionCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn CompletionHooks,
) -> Result<TextCompletionOutcome, AzureTextError>
where
    Source: ClientSource,
{
    let (Some(model), Some(messages)) = (call.model, call.messages) else {
        return Err(AzureTextError::raised(
            422,
            MISSING_MODEL_OR_MESSAGES.to_owned(),
        ));
    };
    let retries = Retries::pop(call.optional_params);
    let prompt =
        super::prompt::resolve_prompt(runtime.prompts, model, messages, runtime.now).await?;
    let supplied = supplied_client(source, call, model, &retries)?;
    let data = request::build_data(
        call.model,
        &prompt,
        call.optional_params,
        call.api_base
            .is_some_and(|base| base.contains(request::CLOUDFLARE_MARKER)),
    );
    let sent = SentCall {
        model: model.to_owned(),
        data,
        prompt,
        max_retries: retries,
        litellm_params: &call.litellm_params,
        supplied,
    };
    dispatch(source, sent, call, runtime, hooks).await
}

// async streams on a literal true like the sync branch, and the sync stream helper never sees litellm_params
async fn dispatch<Source>(
    source: &Source,
    mut sent: SentCall<'_, Source::Client>,
    call: &CompletionCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn CompletionHooks,
) -> Result<TextCompletionOutcome, AzureTextError>
where
    Source: ClientSource,
{
    let stream_requested = matches!(sent.data.get(request::STREAM_KEY), Some(Json::Bool(true)));
    if call.acompletion && stream_requested {
        let stream = streaming::streaming(source, &mut sent, call, runtime, hooks).await?;
        return Ok(TextCompletionOutcome::Stream(stream));
    }
    if call.acompletion {
        let reply = buffered::acompletion(source, &sent, call, runtime, hooks).await?;
        return Ok(TextCompletionOutcome::Buffered(Box::new(reply)));
    }
    if stream_requested {
        let empty = LiteLlmParams::default();
        let mut sent_without_params = SentCall {
            litellm_params: &empty,
            ..sent
        };
        let stream =
            streaming::streaming(source, &mut sent_without_params, call, runtime, hooks).await?;
        return Ok(TextCompletionOutcome::Stream(stream));
    }
    let reply = buffered::complete_buffered(source, &sent, call, runtime, hooks).await?;
    Ok(TextCompletionOutcome::Buffered(Box::new(reply)))
}

// the cloudflare branch swaps the caller's client for one whose base url carries the deployment
fn supplied_client<Source>(
    source: &Source,
    call: &CompletionCall<'_, Source::Client>,
    model: &str,
    retries: &Retries,
) -> Result<Option<ClientHandle<Source::Client>>, AzureTextError>
where
    Source: ClientSource,
{
    let gateway_base = call
        .api_base
        .filter(|base| base.contains(request::CLOUDFLARE_MARKER));
    let (Some(api_base), None) = (gateway_base, call.supplied.as_ref()) else {
        return Ok(call.supplied.clone());
    };
    let handle = source
        .gateway(GatewayClient {
            api_base,
            model,
            api_version: call.api_version,
            max_retries: retries.clone(),
            timeout: call.timeout,
            litellm_params: &call.litellm_params,
            cloudflare_params: &call.cloudflare_params,
            api_key: call.api_key,
            azure_ad_token: call.azure_ad_token,
            azure_ad_token_provider: call.azure_ad_token_provider,
            acompletion: call.acompletion,
        })
        .map_err(|error| AzureTextError::from_endpoint(&error))?;
    Ok(Some(handle))
}

// get_azure_openai_client(client=...): a supplied client comes back as it is
fn resolve_client<Source>(
    source: &Source,
    call: &CompletionCall<'_, Source::Client>,
    sent: &SentCall<'_, Source::Client>,
    is_async: bool,
) -> Result<ClientHandle<Source::Client>, EndpointError>
where
    Source: ClientSource,
{
    sent.supplied.as_ref().map_or_else(
        || {
            source.get(GetClient {
                api_key: call.api_key,
                api_base: call.api_base,
                api_version: call.api_version,
                model: call.model,
                litellm_params: sent.litellm_params,
                is_async,
            })
        },
        |handle| Ok(handle.clone()),
    )
}

// the isinstance guard each path runs on the client it got back
fn require_client<Client>(
    client: ClientHandle<Client>,
    kind: ClientKind,
    message: &str,
) -> Result<ClientHandle<Client>, AzureTextError> {
    if client.kind == kind {
        Ok(client)
    } else {
        Err(AzureTextError::raised(500, message.to_owned()))
    }
}
