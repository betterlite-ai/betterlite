use std::iter::once;
use std::panic::resume_unwind;
use std::thread::scope;
use std::time::Instant;

use betterlite_http::interactions::base::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;
use betterlite_openai::openai_base_config::Headers;

use super::clients::DecisionsAsyncTransport;
use super::error::{Attempt, DecisionsSendError, send_failure};
use super::format::DecisionsOutput;
use super::request;
use super::send::{DecisionsSender, finish, plain, signing_context};
use crate::api::vector_store_http_write::service::ClientKind;

impl<Clients: DecisionsAsyncTransport> DecisionsSender<'_, Clients> {
    // adecisions(): signing off the request task only for AWS, then the provider's async client
    pub(crate) async fn asend_with<T>(
        &mut self,
        call: &DecisionsCall<'_>,
        config: &T,
    ) -> Result<DecisionsOutput, DecisionsSendError>
    where
        T: DecisionsProviderConfig + Sync,
    {
        let prepared = request::prepare(config, call, self.env)?;
        let context = signing_context(call, &prepared);
        let signed = if config.signs_with_aws(&call.api_base) {
            signed_off_task(|| config.sign_request(prepared.outbound_headers.clone(), context))
        } else {
            config.sign_request(prepared.outbound_headers.clone(), context)
        };
        let params: Params = once((
            String::from("custom_llm_provider"),
            JsonValue::Text(call.custom_llm_provider.clone()),
        ))
        .collect();
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

// run_aws_signing: the signing call runs on the executor's thread, not the request task
fn signed_off_task<F>(sign: F) -> (Headers, Option<Vec<u8>>)
where
    F: FnOnce() -> (Headers, Option<Vec<u8>>) + Send,
{
    scope(|threads| match threads.spawn(sign).join() {
        Ok(signed) => signed,
        Err(payload) => resume_unwind(payload),
    })
}
