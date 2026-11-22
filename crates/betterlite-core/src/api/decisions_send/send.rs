use std::time::Instant;

use betterlite_http::interactions::base::Params;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::transformation::provider_config::SigningContext;
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::Redactor;

use super::clients::{DecisionsAsyncClient, DecisionsClients, DecisionsSyncClient};
use super::config::{self, DecisionsSendConfig};
use super::error::{DecisionsSendError, send_failure, status_failure};
use super::format::{DecisionsOutput, format_response};
use super::request::{self, Prepared};
use crate::api::vector_store_http_write::service::ClientKind;

// The send stage: B173's pooled clients, B249's logging environment, and the call B3513 prepared
pub struct DecisionsSender<'env, Clients: DecisionsClients = HttpClients> {
    clients: &'env mut Clients,
    env: &'env PrePostEnv<'env>,
}

impl<'env, Clients: DecisionsClients> DecisionsSender<'env, Clients> {
    pub fn new(clients: &'env mut Clients, env: &'env PrePostEnv<'env>) -> Self {
        Self { clients, env }
    }

    pub fn send(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        self.send_with(call, config::of(&call.provider_config))
    }

    pub async fn asend(
        &mut self,
        call: &DecisionsCall<'_>,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        self.asend_with(call, config::of(&call.provider_config))
            .await
    }

    // decisions(): inline signing, the pool's default client, one blocking POST
    pub(crate) fn send_with(
        &mut self,
        call: &DecisionsCall<'_>,
        config: &dyn DecisionsSendConfig,
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
        let outcome = client.send_blocking(wire);
        let reply = outcome.map_err(|error| {
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
    pub(crate) async fn asend_with(
        &mut self,
        call: &DecisionsCall<'_>,
        config: &dyn DecisionsSendConfig,
    ) -> Result<DecisionsOutput, DecisionsSendError> {
        let prepared = request::prepare(config, call, self.env)?;
        let context = signing_context(call, &prepared);
        let signed = if config.signs_with_aws(&call.api_base) {
            signed_off_task(|| config.sign_request(prepared.outbound_headers.clone(), context))
        } else {
            config.sign_request(prepared.outbound_headers.clone(), context)
        };
        let params = provider_params(&call.custom_llm_provider);
        let client = self
            .clients
            .select_async(&params)
            .map_err(|error| super::error::plain(call, self.env.redactor, Box::new(error)))?;
        let wire = request::wire(call, self.env.redactor, &prepared, signed)?;
        let started = Instant::now();
        let outcome = client.send(wire).await;
        let reply = outcome.map_err(|error| {
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
fn finish(
    config: &dyn DecisionsSendConfig,
    call: &DecisionsCall<'_>,
    redactor: &Redactor,
    reply: &UpstreamResponse,
) -> Result<DecisionsOutput, DecisionsSendError> {
    if reply.status >= 400 {
        return Err(status_failure(config, call, redactor, reply));
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

// get_async_httpx_client(llm_provider=litellm.LlmProviders(custom_llm_provider))
fn provider_params(provider: &str) -> Params {
    let mut params = Params::new();
    params.insert(
        String::from("custom_llm_provider"),
        JsonValue::Text(provider.to_owned()),
    );
    params
}
