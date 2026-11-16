use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use secrecy::SecretString;

use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{ClientKind, LiteLlmParams};

use super::arguments::{CompletionCall, Runtime, SentCall};
use super::client::{ClientHandle, ClientSource, GatewayClient, GetClient};
use super::error::{AzureTextError, MISSING_MODEL_OR_MESSAGES};
use super::logging::CompletionHooks;
use super::request::{self, Retries};
use super::response::ChatReply;

mod buffered;
mod records;
mod streaming;

// litellm's AzureTextCompletion for the text completion deployment
pub struct AzureTextCompletion<Source: ClientSource> {
    source: Source,
}

pub enum TextCompletionOutcome {
    Buffered(ChatReply),
    Stream(CompletionStream),
}

impl<Source: ClientSource> AzureTextCompletion<Source> {
    #[must_use]
    pub const fn new(source: Source) -> Self {
        Self { source }
    }

    #[must_use]
    pub fn validate_environment(
        api_key: Option<&SecretString>,
        azure_ad_token: Option<&SecretString>,
    ) -> SecretHeaders {
        request::validate_environment(api_key, azure_ad_token)
    }

    // AzureTextCompletion.completion: the prompt comes from prompt_factory on every call
    pub async fn completion(
        &self,
        call: CompletionCall<'_, Source::Client>,
        runtime: &Runtime<'_>,
        hooks: &mut dyn CompletionHooks,
    ) -> Result<TextCompletionOutcome, AzureTextError> {
        let (Some(model), Some(messages)) = (call.model, call.messages) else {
            return Err(AzureTextError::raised(422, MISSING_MODEL_OR_MESSAGES));
        };
        let retries = Retries::pop(call.optional_params);
        let prompt =
            super::prompt::resolve_prompt(runtime.prompts, model, messages, runtime.now).await?;
        let supplied = self.supplied_client(&call, model, &retries)?;
        let data = request::build_data(
            call.model,
            &prompt,
            call.optional_params,
            request::is_cloudflare(call.api_base),
        );
        let sent = SentCall {
            model: model.to_owned(),
            data,
            prompt,
            max_retries: retries,
            litellm_params: &call.litellm_params,
            supplied,
        };
        if call.acompletion {
            if request::optional_stream(call.optional_params) {
                let stream = self.async_streaming(&sent, &call, runtime, hooks).await?;
                return Ok(TextCompletionOutcome::Stream(stream));
            }
            let reply = self.acompletion(&sent, &call, runtime, hooks).await?;
            return Ok(TextCompletionOutcome::Buffered(reply));
        }
        if request::literal_true_stream(call.optional_params) {
            // completion() calls streaming() without litellm_params, so its client sees an empty dict
            let empty = LiteLlmParams::default();
            let mut sent = SentCall {
                litellm_params: &empty,
                ..sent
            };
            let stream = self
                .streaming(&mut sent, &call, runtime, hooks)
                .await
                .map_err(super::error::AzureTextFailure::into_azure)?;
            return Ok(TextCompletionOutcome::Stream(stream));
        }
        let reply = self.complete_buffered(&sent, &call, runtime, hooks).await?;
        Ok(TextCompletionOutcome::Buffered(reply))
    }

    // the cloudflare branch swaps the caller's client for one whose base url carries the deployment
    fn supplied_client(
        &self,
        call: &CompletionCall<'_, Source::Client>,
        model: &str,
        retries: &Retries,
    ) -> Result<Option<ClientHandle<Source::Client>>, AzureTextError> {
        let Some(api_base) = call
            .api_base
            .filter(|base| request::is_cloudflare(Some(base)))
        else {
            return Ok(call.supplied.clone());
        };
        if call.supplied.is_some() {
            return Ok(call.supplied.clone());
        }
        let handle = self
            .source
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
            .map_err(|source| AzureTextError::from_endpoint(&source))?;
        Ok(Some(handle))
    }

    // get_azure_openai_client(client=...): a supplied client comes back as it is
    fn resolve_client(
        &self,
        call: &CompletionCall<'_, Source::Client>,
        litellm_params: &LiteLlmParams,
        supplied: Option<&ClientHandle<Source::Client>>,
        is_async: bool,
    ) -> Result<ClientHandle<Source::Client>, EndpointError> {
        if let Some(handle) = supplied {
            return Ok(handle.clone());
        }
        self.source.get(GetClient {
            api_key: call.api_key,
            api_base: call.api_base,
            api_version: call.api_version,
            model: call.model,
            litellm_params,
            is_async,
        })
    }

    // the isinstance guard each path runs on the client it got back
    fn require_client(
        client: ClientHandle<Source::Client>,
        kind: ClientKind,
        message: &str,
    ) -> Result<ClientHandle<Source::Client>, AzureTextError> {
        if client.kind == kind {
            return Ok(client);
        }
        Err(AzureTextError::raised(500, message))
    }
}
