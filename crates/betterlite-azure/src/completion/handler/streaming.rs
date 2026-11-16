use betterlite_openai::text_completion_transport::sdk::ParsedCompletion;
use betterlite_openai::text_completion_transport::stream::CompletionStream;

use crate::endpoints::projection::ClientKind;

use super::super::arguments::{CompletionCall, Runtime, SentCall};
use super::super::client::ClientSource;
use super::super::error::{
    ASYNC_CLIENT, AzureTextError, AzureTextFailure, BUFFERED_ANSWER, MAX_RETRIES_TYPE, SYNC_CLIENT,
};
use super::super::logging::CompletionHooks;
use super::super::request::Retries;
use super::super::stream::{StreamInput, wrap};
use super::{AzureTextCompletion, records};

impl<Source: ClientSource> AzureTextCompletion<Source> {
    // the direct sync stream pops the retry count off the data it was handed, then returns the wrapper
    pub async fn streaming(
        &self,
        sent: &mut SentCall<'_, Source::Client>,
        call: &CompletionCall<'_, Source::Client>,
        runtime: &Runtime<'_>,
        hooks: &mut dyn CompletionHooks,
    ) -> Result<CompletionStream, AzureTextFailure> {
        let retries = Retries::pop(&mut sent.data);
        if !retries.is_int() {
            return Err(AzureTextFailure::Azure(AzureTextError::raised(
                422,
                MAX_RETRIES_TYPE,
            )));
        }
        let client = self
            .resolve_client(call, sent.litellm_params, sent.supplied.as_ref(), false)
            .map_err(AzureTextFailure::Endpoint)?;
        if client.kind != ClientKind::AzureOpenAi {
            return Err(AzureTextFailure::Azure(AzureTextError::raised(
                500,
                SYNC_CLIENT,
            )));
        }
        hooks
            .pre_call(records::async_pre_call(&client, &sent.data))
            .map_err(AzureTextFailure::Logging)?;
        let raw = client
            .transport
            .create(&sent.data, call.timeout)
            .await
            .map_err(AzureTextFailure::Raw)?;
        let parsed = raw.parse().map_err(AzureTextFailure::Raw)?;
        let ParsedCompletion::Stream(bytes) = parsed else {
            return Err(AzureTextFailure::Azure(AzureTextError::internal(
                BUFFERED_ANSWER,
            )));
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

    // async_streaming wraps every exception, including the ones the sync stream lets through
    pub async fn async_streaming(
        &self,
        sent: &SentCall<'_, Source::Client>,
        call: &CompletionCall<'_, Source::Client>,
        runtime: &Runtime<'_>,
        hooks: &mut dyn CompletionHooks,
    ) -> Result<CompletionStream, AzureTextError> {
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
        let ParsedCompletion::Stream(bytes) = parsed else {
            return Err(AzureTextError::internal(BUFFERED_ANSWER));
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
}
