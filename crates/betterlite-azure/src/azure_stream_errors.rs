pub mod content_filter;
pub mod error;
pub mod prepare;
pub mod records;
pub mod stream;

mod tests;

use crate::credentials::headers::{HeaderLookup, process_azure_headers};
use crate::credentials::source::CredentialSources;

use self::error::{AzureStreamFailure, Raised, StreamingError};
use self::records::{GetStreamClient, StreamCall, StreamHooks, StreamRuntime, StreamSource};
use self::stream::{AzureChatStream, WrapperHeaders};

const fn get_client<'call, 'get>(
    call: &'get StreamCall<'call>,
    is_async: bool,
) -> GetStreamClient<'get>
where
    'call: 'get,
{
    GetStreamClient {
        api_version: call.api_version,
        api_base: call.api_base,
        api_key: call.api_key,
        model: call.model,
        supplied: call.supplied,
        litellm_params: call.litellm_params,
        is_async,
    }
}

// AzureChatCompletion.streaming: the unused client parameters are assembled first and then dropped
pub async fn streaming<Source>(
    source: &Source,
    sources: &dyn CredentialSources,
    call: &StreamCall<'_>,
    hooks: &mut dyn StreamHooks,
    runtime: StreamRuntime,
) -> Result<AzureChatStream, StreamingError>
where
    Source: StreamSource,
{
    let unused = prepare::unused_client_params(call, sources)?;
    let client = source.get(get_client(call, false))?;
    prepare::require_stream_client(&client, false)?;
    let token = unused.logged_token().or(call.azure_ad_token);
    hooks.pre_call(prepare::pre_call(call, &client, token)?)?;
    let answer = client.transport.create(call.data, call.timeout).await?;
    let raw = stream::raw_headers(&answer.headers);
    hooks.response_headers(&raw);
    let pairs: Vec<(String, String)> = raw
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let headers = WrapperHeaders::Processed(process_azure_headers(&pairs, HeaderLookup::Exact));
    Ok(stream::wrap_stream(
        answer.body,
        call,
        hooks.stream_defaults(),
        headers,
        runtime,
    ))
}

// AzureChatCompletion.async_streaming: every failure inside the try becomes an AzureOpenAIError
pub async fn async_streaming<Source>(
    source: &Source,
    call: &StreamCall<'_>,
    hooks: &mut dyn StreamHooks,
    runtime: StreamRuntime,
) -> Result<AzureChatStream, StreamingError>
where
    Source: StreamSource,
{
    match async_inner(source, call, hooks, runtime).await {
        Ok(stream) => Ok(stream),
        Err(raised) => Err(StreamingError::Azure(AzureStreamFailure::wrapped(raised))),
    }
}

async fn async_inner<Source>(
    source: &Source,
    call: &StreamCall<'_>,
    hooks: &mut dyn StreamHooks,
    runtime: StreamRuntime,
) -> Result<AzureChatStream, Raised>
where
    Source: StreamSource,
{
    let client = source.get(get_client(call, true))?;
    prepare::require_stream_client(&client, true)?;
    hooks
        .pre_call(prepare::pre_call(call, &client, call.azure_ad_token)?)
        .map_err(|error| Raised::plain(error.to_string()))?;
    let answer = client.transport.create(call.data, call.timeout).await?;
    let raw = stream::raw_headers(&answer.headers);
    hooks.response_headers(&raw);
    Ok(stream::wrap_stream(
        answer.body,
        call,
        hooks.stream_defaults(),
        WrapperHeaders::Raw(raw),
        runtime,
    ))
}
