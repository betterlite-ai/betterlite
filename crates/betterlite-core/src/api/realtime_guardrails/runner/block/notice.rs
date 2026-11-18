use std::error::Error;

use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use indexmap::IndexMap;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrails::error::RealtimeGuardrailsError;

use super::{RealtimeGuardrails, UnitFuture};

impl<Config, CallbackError> RealtimeGuardrails<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    // LiteLLM sends the notice straight to the client socket; it is not an event, so it gets no normalization or beta translation
    pub(super) fn send_client_notice<'call>(
        &'call mut self,
        message: &'call Json,
    ) -> UnitFuture<'call, Config::Error, CallbackError> {
        async move {
            let error = Json::Object(IndexMap::from([
                (
                    String::from("type"),
                    Json::Text(String::from("guardrail_violation")),
                ),
                (String::from("message"), message.clone()),
                (
                    String::from("code"),
                    Json::Text(String::from("content_policy_violation")),
                ),
            ]));
            let notice = Json::Object(IndexMap::from([
                (String::from("type"), Json::Text(String::from("error"))),
                (String::from("error"), error),
            ]));
            let text = serde_json::to_string(&notice).map_err(RealtimeGuardrailsError::Encode)?;
            let client = self
                .state
                .client
                .as_mut()
                .ok_or(RealtimeGuardrailsError::ClientMissing)?;
            client
                .send_text(&text)
                .await
                .map_err(RealtimeGuardrailsError::Client)
        }
        .boxed_local()
    }
}
