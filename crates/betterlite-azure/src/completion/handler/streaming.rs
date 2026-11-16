use betterlite_openai::text_completion_transport::sdk::ParsedCompletion;
use betterlite_openai::text_completion_transport::stream::CompletionStream;

use crate::endpoints::projection::ClientKind;

use super::super::arguments::{CompletionCall, Runtime, SentCall};
use super::super::client::{ClientSource, CompletionsTransport};
use super::super::error::{
    ASYNC_CLIENT, AzureTextError, BUFFERED_ANSWER, MAX_RETRIES_TYPE, SYNC_CLIENT,
};
use super::super::logging::CompletionHooks;
use super::super::request::Retries;
use super::super::stream::{StreamInput, wrap};
use super::{records, require_client, resolve_client};

// the sync stream pops and checks the retry count off the data it was handed, the async one wraps every failure
pub async fn streaming<Source>(
    source: &Source,
    sent: &mut SentCall<'_, Source::Client>,
    call: &CompletionCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn CompletionHooks,
) -> Result<CompletionStream, AzureTextError>
where
    Source: ClientSource,
{
    if !call.acompletion && !Retries::pop(&mut sent.data).is_int() {
        return Err(AzureTextError::raised(422, MAX_RETRIES_TYPE.to_owned()));
    }
    let (kind, message) = if call.acompletion {
        (ClientKind::AsyncAzureOpenAi, ASYNC_CLIENT)
    } else {
        (ClientKind::AzureOpenAi, SYNC_CLIENT)
    };
    let resolved = resolve_client(source, call, sent, call.acompletion)
        .map_err(|error| AzureTextError::from_endpoint(&error))?;
    let client = require_client(resolved, kind, message)?;
    hooks
        .pre_call(records::async_pre_call(&client, &sent.data))
        .map_err(|error| AzureTextError::raised(500, error.to_string()))?;
    let raw = client
        .transport
        .create(&sent.data, call.timeout)
        .await
        .map_err(|error| AzureTextError::wrapped(&error))?;
    let parsed = raw
        .parse()
        .map_err(|error| AzureTextError::wrapped(&error))?;
    let ParsedCompletion::Stream(bytes) = parsed else {
        return Err(AzureTextError::raised(500, BUFFERED_ANSWER.to_owned()));
    };
    let input = StreamInput {
        model: &sent.model,
        data: &sent.data,
        defaults: hooks.stream_defaults(),
        now: runtime.stream_now,
        next_id: runtime.stream_id,
    };
    Ok(wrap(bytes, input))
}
