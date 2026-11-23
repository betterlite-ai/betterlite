use std::time::Instant;

use betterlite_http::interactions::base::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;

use super::clients::DecisionsAsyncTransport;
use super::error::{Attempt, DecisionsSendError, send_failure};
use super::format::DecisionsOutput;
use super::request;
use super::send::{DecisionsSender, finish, plain, signing_context};
use super::signing::SigningSeam;
use crate::api::vector_store_http_write::service::ClientKind;

impl<Clients: DecisionsAsyncTransport> DecisionsSender<'_, Clients> {
    // adecisions(): signing off the request task only for AWS, then the provider's async client
    pub(crate) async fn asend_with<T, S>(
        &mut self,
        call: &DecisionsCall<'_>,
        config: &T,
        signing: &S,
    ) -> Result<DecisionsOutput, DecisionsSendError>
    where
        T: DecisionsProviderConfig + Sync,
        S: SigningSeam,
    {
        let prepared = request::prepare(config, call, self.env)?;
        let sign = || {
            config.sign_request(
                prepared.outbound_headers.clone(),
                signing_context(call, &prepared),
            )
        };
        let signed = signing
            .sign(config.signs_with_aws(&call.api_base), &sign)
            .await;
        let params = Params::from([(
            String::from("custom_llm_provider"),
            JsonValue::Text(call.custom_llm_provider.clone()),
        )]);
        let wire = request::wire(call, self.env.redactor, &prepared, signed)?;
        let started = Instant::now();
        let reply = self
            .clients
            .send_async(&params, wire)
            .map_err(|error| plain(call, self.env.redactor, Box::new(error)))?
            .await
            .map_err(|error| {
                let attempt = Attempt {
                    mode: ClientKind::Asynchronous,
                    elapsed: started.elapsed(),
                };
                send_failure(config, call, self.env.redactor, error, attempt)
            })?;
        finish(config, call, self.env.redactor, &reply)
    }
}
