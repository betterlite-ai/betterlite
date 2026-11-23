use std::error::Error;
use std::time::Instant;

use betterlite_error::dispatch::{DispatchInput, dispatch};
use betterlite_http::interactions::base::Params;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::call::provider::ProviderConfig;
use betterlite_openai::decisions::transformation::provider_config::{
    DecisionsProviderConfig, SigningContext,
};
use betterlite_redact::secret::Redactor;

use super::clients::{DecisionsClients, DecisionsSyncClient};
use super::error::{DecisionsSendError, mapped, mapped_failure, raw_exception, send_failure};
use super::format::{DecisionsOutput, format_response};
use super::request::{self, Prepared};
use crate::api::vector_store_http_write::error::status_facts;
use crate::api::vector_store_http_write::service::ClientKind;

pub struct DecisionsSender<'env, Clients: DecisionsClients = HttpClients> {
    pub(super) clients: &'env mut Clients,
    pub(super) env: &'env PrePostEnv<'env>,
}

// ProviderConfigManager.get_provider_decisions_config boxes one config per provider, and the call runs on that typing
macro_rules! with_config {
    ($call:expr, |$config:ident| $send:expr) => {
        match &$call.provider_config {
            ProviderConfig::Openai($config) => $send,
            ProviderConfig::Openrouter($config) => $send,
            ProviderConfig::AzureAi($config) => $send,
            ProviderConfig::Perplexity($config) => $send,
            ProviderConfig::Typesafe($config) => $send,
            ProviderConfig::StrandsDecider($config) => $send,
            ProviderConfig::Cloudflare($config) => $send,
            ProviderConfig::HostedVllm($config) => $send,
        }
    };
}

impl<'env, Clients: DecisionsClients> DecisionsSender<'env, Clients> {
    // ast-grep-ignore: no-one-line-functions - callers hand in the pool and the environment, and the fields stay private
    pub fn new(clients: &'env mut Clients, env: &'env PrePostEnv<'env>) -> Self {
        Self { clients, env }
    }

    // ast-grep-ignore: no-one-line-functions - B3515 calls these two entry points from its own crate, so the dispatch stays a method
    pub fn send(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        with_config!(call, |config| self.send_with(call, config.as_ref()))
    }

    // ast-grep-ignore: no-one-line-functions - the async twin of send, with the same dispatcher an inliner cannot reach
    pub async fn asend(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        with_config!(call, |config| self.asend_with(call, config.as_ref()).await)
    }

    // decisions(): inline signing, the pool's default client, one blocking POST
    pub(crate) fn send_with<T: DecisionsProviderConfig>(
        &mut self,
        call: &DecisionsCall<'_>,
        config: &T,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        let prepared = request::prepare(config, call, self.env)?;
        let signed = config.sign_request(
            prepared.outbound_headers.clone(),
            signing_context(call, &prepared),
        );
        let client = self
            .clients
            .select_sync(&Params::new())
            .map_err(|error| plain(call, self.env.redactor, Box::new(error)))?;
        let wire = request::wire(call, self.env.redactor, &prepared, signed)?;
        let started = Instant::now();
        let reply = client.send_blocking(wire).map_err(|error| {
            send_failure(
                config,
                call,
                self.env.redactor,
                error,
                ClientKind::Sync,
                started.elapsed(),
            )
        })?;
        finish(config, call, self.env.redactor, &reply)
    }
}

// A failure outside the provider's own class - building the client, serializing the body, an unusable header or url - goes to exception_type's catchall
pub(super) fn plain(
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    error: Box<dyn Error + Send + Sync>,
) -> DecisionsSendError {
    let raw = raw_exception(call, error.to_string(), None, false, None, "");
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    mapped(class, error)
}

// raise_for_status, then transform_decisions_response, then _format_response
pub(super) fn finish<T: DecisionsProviderConfig>(
    config: &T,
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    reply: &UpstreamResponse,
) -> Result<DecisionsOutput, DecisionsSendError> {
    if reply.status >= 400 {
        return Err(mapped_failure(config, call, redactor, &status_facts(reply)));
    }
    let response = config
        .transform_decisions_response(
            &call.model,
            &call.custom_llm_provider,
            &reply.body,
            &call.ir_request,
        )
        .map_err(|error| super::error::transform_error(call, redactor, error))?;
    format_response(config, call, &response).map_err(DecisionsSendError::from)
}

pub(super) fn signing_context<'call>(
    call: &'call DecisionsCall<'_>,
    prepared: &'call Prepared,
) -> SigningContext<'call> {
    SigningContext {
        url: &prepared.url,
        api_base: &call.api_base,
        body: &call.body,
        api_key: call.api_key.as_ref(),
        litellm_params: &call.litellm_params,
    }
}
