use std::time::Instant;

use betterlite_http::interactions::base::Params;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::call::provider::ProviderConfig;
use betterlite_openai::decisions::transformation::provider_config::{
    DecisionsProviderConfig, SigningContext,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::Redactor;

use super::clients::{DecisionsAsyncClient, DecisionsClients, DecisionsSyncClient};
use super::error::{DecisionsSendError, mapped_failure, send_failure};
use super::format::{DecisionsOutput, format_response};
use super::request::{self, Prepared};
use crate::api::vector_store_http_write::error::status_facts;
use crate::api::vector_store_http_write::service::ClientKind;

pub struct DecisionsSender<'env, Clients: DecisionsClients = HttpClients> {
    clients: &'env mut Clients,
    env: &'env PrePostEnv<'env>,
}

impl<'env, Clients: DecisionsClients> DecisionsSender<'env, Clients> {
    // ast-grep-ignore: no-one-line-functions - callers hand in the pool and the environment, and the fields stay private
    pub fn new(clients: &'env mut Clients, env: &'env PrePostEnv<'env>) -> Self {
        Self { clients, env }
    }

    // ProviderConfigManager.get_provider_decisions_config boxes one config per provider
    pub fn send(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        match &call.provider_config {
            ProviderConfig::Openai(config) => self.send_with(call, config.as_ref()),
            ProviderConfig::Openrouter(config) => self.send_with(call, config.as_ref()),
            ProviderConfig::AzureAi(config) => self.send_with(call, config.as_ref()),
            ProviderConfig::Perplexity(config) => self.send_with(call, config.as_ref()),
            ProviderConfig::Typesafe(config) => self.send_with(call, config.as_ref()),
            ProviderConfig::StrandsDecider(config) => self.send_with(call, config.as_ref()),
            ProviderConfig::Cloudflare(config) => self.send_with(call, config.as_ref()),
            ProviderConfig::HostedVllm(config) => self.send_with(call, config.as_ref()),
        }
    }

    pub async fn asend(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        match &call.provider_config {
            ProviderConfig::Openai(config) => self.asend_with(call, config.as_ref()).await,
            ProviderConfig::Openrouter(config) => self.asend_with(call, config.as_ref()).await,
            ProviderConfig::AzureAi(config) => self.asend_with(call, config.as_ref()).await,
            ProviderConfig::Perplexity(config) => self.asend_with(call, config.as_ref()).await,
            ProviderConfig::Typesafe(config) => self.asend_with(call, config.as_ref()).await,
            ProviderConfig::StrandsDecider(config) => self.asend_with(call, config.as_ref()).await,
            ProviderConfig::Cloudflare(config) => self.asend_with(call, config.as_ref()).await,
            ProviderConfig::HostedVllm(config) => self.asend_with(call, config.as_ref()).await,
        }
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
            .map_err(|error| super::error::plain(call, self.env.redactor, Box::new(error)))?;
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

    // adecisions(): signing off the request task only for AWS, then the provider's async client
    pub(crate) async fn asend_with<T: DecisionsProviderConfig + Sync>(
        &mut self,
        call: &DecisionsCall<'_>,
        config: &T,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        let prepared = request::prepare(config, call, self.env)?;
        let context = signing_context(call, &prepared);
        let signed = if config.signs_with_aws(&call.api_base) {
            signed_off_task(|| config.sign_request(prepared.outbound_headers.clone(), context))
        } else {
            config.sign_request(prepared.outbound_headers.clone(), context)
        };
        let params: Params = std::iter::once((
            String::from("custom_llm_provider"),
            JsonValue::Text(call.custom_llm_provider.clone()),
        ))
        .collect();
        let client = self
            .clients
            .select_async(&params)
            .map_err(|error| super::error::plain(call, self.env.redactor, Box::new(error)))?;
        let wire = request::wire(call, self.env.redactor, &prepared, signed)?;
        let started = Instant::now();
        let reply = client.send(wire).await.map_err(|error| {
            send_failure(
                config,
                call,
                self.env.redactor,
                error,
                ClientKind::Asynchronous,
                started.elapsed(),
            )
        })?;
        finish(config, call, self.env.redactor, &reply)
    }
}

// raise_for_status, then transform_decisions_response, then _format_response
fn finish<T: DecisionsProviderConfig>(
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

fn signing_context<'call>(
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

// run_aws_signing: the signing call runs on the executor's thread, not the request task
fn signed_off_task<F>(sign: F) -> (Headers, Option<Vec<u8>>)
where
    F: FnOnce() -> (Headers, Option<Vec<u8>>) + Send,
{
    std::thread::scope(|scope| match scope.spawn(sign).join() {
        Ok(signed) => signed,
        Err(payload) => std::panic::resume_unwind(payload),
    })
}
