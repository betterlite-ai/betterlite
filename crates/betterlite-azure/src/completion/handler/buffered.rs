use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::sdk::ParsedCompletion;

use crate::endpoints::projection::ClientKind;

use super::super::arguments::{CompletionCall, Runtime, SentCall};
use super::super::client::ClientSource;
use super::super::error::{
    ASYNC_CLIENT, AzureTextError, MAX_RETRIES_TYPE, STREAM_ANSWER, SYNC_CLIENT,
};
use super::super::logging::CompletionHooks;
use super::super::response::{ChatReply, ProjectionError};
use super::{AzureTextCompletion, records};

fn internal(source: &serde_json::Error) -> AzureTextError {
    AzureTextError::internal(source.to_string())
}

impl<Source: ClientSource> AzureTextCompletion<Source> {
    // the sync buffered call logs before it validates the retry count, then converts the parsed reply
    pub(super) async fn complete_buffered(
        &self,
        sent: &SentCall<'_, Source::Client>,
        call: &CompletionCall<'_, Source::Client>,
        runtime: &Runtime<'_>,
        hooks: &mut dyn CompletionHooks,
    ) -> Result<ChatReply, AzureTextError> {
        hooks
            .pre_call(records::sync_pre_call(call, &sent.prompt, &sent.data))
            .map_err(|source| AzureTextError::from_logging(&source))?;
        if !sent.max_retries.is_int() {
            return Err(AzureTextError::raised(422, MAX_RETRIES_TYPE));
        }
        let client = self
            .resolve_client(call, sent.litellm_params, sent.supplied.as_ref(), false)
            .map_err(|source| AzureTextError::from_endpoint(&source))?;
        let client = Self::require_client(client, ClientKind::AzureOpenAi, SYNC_CLIENT)?;
        let raw = client
            .transport
            .create(&sent.data, call.timeout)
            .await
            .map_err(|source| AzureTextError::wrapped(&source))?;
        let parsed = raw
            .parse()
            .map_err(|source| AzureTextError::wrapped(&source))?;
        let ParsedCompletion::Buffered(reply) = parsed else {
            return Err(AzureTextError::internal(STREAM_ANSWER));
        };
        let dumped = serde_json::to_value(reply.as_ref()).map_err(|source| internal(&source))?;
        let dumped: Json = serde_json::from_value(dumped).map_err(|source| internal(&source))?;
        hooks.post_call(records::sync_post_call(call, &sent.prompt, dumped));
        super::super::response::project(reply.as_ref(), runtime.identifier, runtime.clock)
            .map_err(ProjectionError::into_azure)
    }

    // the async buffered call converts the SDK model dump directly and logs no post call
    pub async fn acompletion(
        &self,
        sent: &SentCall<'_, Source::Client>,
        call: &CompletionCall<'_, Source::Client>,
        runtime: &Runtime<'_>,
        hooks: &mut dyn CompletionHooks,
    ) -> Result<ChatReply, AzureTextError> {
        let client = self
            .resolve_client(call, sent.litellm_params, sent.supplied.as_ref(), true)
            .map_err(|source| AzureTextError::from_endpoint(&source))?;
        let client = Self::require_client(client, ClientKind::AsyncAzureOpenAi, ASYNC_CLIENT)?;
        hooks
            .pre_call(records::async_pre_call(&client, &sent.data))
            .map_err(|source| AzureTextError::from_logging(&source))?;
        let raw = client
            .transport
            .create(&sent.data, call.timeout)
            .await
            .map_err(|source| AzureTextError::wrapped(&source))?;
        let parsed = raw
            .parse()
            .map_err(|source| AzureTextError::wrapped(&source))?;
        let ParsedCompletion::Buffered(reply) = parsed else {
            return Err(AzureTextError::internal(STREAM_ANSWER));
        };
        super::super::response::project_async(reply.as_ref(), runtime.identifier, runtime.clock)
            .map_err(ProjectionError::into_azure)
    }
}
