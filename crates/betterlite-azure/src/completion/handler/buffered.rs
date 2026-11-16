use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::sdk::ParsedCompletion;

use crate::endpoints::projection::ClientKind;

use super::super::arguments::{CompletionCall, Runtime, SentCall};
use super::super::client::{ClientSource, CompletionsTransport};
use super::super::error::{
    ASYNC_CLIENT, AzureTextError, MAX_RETRIES_TYPE, STREAM_ANSWER, SYNC_CLIENT,
};
use super::super::logging::CompletionHooks;
use super::super::response::{ChatReply, project};
use super::{records, require_client, resolve_client};

// the sync buffered call logs before it validates the retry count, then converts the parsed reply
pub(super) async fn complete_buffered<Source>(
    source: &Source,
    sent: &SentCall<'_, Source::Client>,
    call: &CompletionCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn CompletionHooks,
) -> Result<ChatReply, AzureTextError>
where
    Source: ClientSource,
{
    hooks
        .pre_call(records::sync_pre_call(call, &sent.prompt, &sent.data))
        .map_err(|error| AzureTextError::raised(500, error.to_string()))?;
    if !sent.max_retries.is_int() {
        return Err(AzureTextError::raised(422, MAX_RETRIES_TYPE.to_owned()));
    }
    let resolved = resolve_client(source, call, sent, false)
        .map_err(|error| AzureTextError::from_endpoint(&error))?;
    let client = require_client(resolved, ClientKind::AzureOpenAi, SYNC_CLIENT)?;
    let raw = client
        .transport
        .create(&sent.data, call.timeout)
        .await
        .map_err(|error| AzureTextError::wrapped(&error))?;
    let parsed = raw
        .parse()
        .map_err(|error| AzureTextError::wrapped(&error))?;
    let ParsedCompletion::Buffered(reply) = parsed else {
        return Err(AzureTextError::raised(500, STREAM_ANSWER.to_owned()));
    };
    let dumped_value = serde_json::to_value(reply.as_ref())
        .map_err(|error| AzureTextError::raised(500, error.to_string()))?;
    let dumped_json: Json = serde_json::from_value(dumped_value)
        .map_err(|error| AzureTextError::raised(500, error.to_string()))?;
    hooks.post_call(records::sync_post_call(call, &sent.prompt, dumped_json));
    project(reply.as_ref(), runtime.identifier, runtime.clock, false)
        .map_err(|error| AzureTextError::raised(500, error.to_string()))
}

// the async buffered call converts the SDK model dump directly and logs no post call
pub async fn acompletion<Source>(
    source: &Source,
    sent: &SentCall<'_, Source::Client>,
    call: &CompletionCall<'_, Source::Client>,
    runtime: &Runtime<'_>,
    hooks: &mut dyn CompletionHooks,
) -> Result<ChatReply, AzureTextError>
where
    Source: ClientSource,
{
    let resolved = resolve_client(source, call, sent, true)
        .map_err(|error| AzureTextError::from_endpoint(&error))?;
    let client = require_client(resolved, ClientKind::AsyncAzureOpenAi, ASYNC_CLIENT)?;
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
    let ParsedCompletion::Buffered(reply) = parsed else {
        return Err(AzureTextError::raised(500, STREAM_ANSWER.to_owned()));
    };
    project(reply.as_ref(), runtime.identifier, runtime.clock, true)
        .map_err(|error| AzureTextError::raised(500, error.to_string()))
}
