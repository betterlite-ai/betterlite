use std::collections::BTreeMap;
use std::error::Error;

use betterlite_core::agents::json::AgentJson;
use serde::Deserialize as _;
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::pydantic_ai_transformation::transform::ResponseMessage;
use crate::watsonx_credentials::bearer::{TokenClock, TokenTransport};
use crate::watsonx_credentials::params::WxoLitellmParams;
use crate::watsonx_credentials::token_cache::TokenStore;
use crate::watsonx_transform::extract::WxoSendParams;
use crate::watsonx_transform::fake_streaming::WxoStreamStep;

use super::handler::{RunError, RunHandler};
use super::transport::RunTransport;

mod tests;

#[derive(Debug, Error)]
pub enum ConfigError<E, F>
where
    E: Error + 'static,
    F: Error + 'static,
{
    #[error("litellm_params is required for a watsonx Orchestrate agent")]
    MissingLitellmParams,
    #[error("litellm_params for a watsonx Orchestrate agent is not a mapping")]
    LitellmParams(#[source] serde_json::Error),
    #[error(transparent)]
    Run(#[from] RunError<E, F>),
}

pub struct ConfigCall<'call> {
    pub request_id: &'call str,
    pub params: &'call WxoSendParams,
    pub kwargs: &'call BTreeMap<String, AgentJson>,
}

pub struct WatsonxRunConfig<T, K, C, S, U> {
    pub handler: RunHandler<T, K, C, S, U>,
}

impl<T, K, C, S, U> WatsonxRunConfig<T, K, C, S, U>
where
    T: RunTransport,
    K: TokenTransport,
    C: TokenClock,
    S: TokenStore,
    U: EventSource + Send + Sync,
{
    pub async fn handle_non_streaming(
        &mut self,
        call: &ConfigCall<'_>,
    ) -> Result<Event<ResponseMessage>, ConfigError<T::Error, K::Error>> {
        let raw = call
            .kwargs
            .get("litellm_params")
            .ok_or(ConfigError::MissingLitellmParams)?;
        let params = WxoLitellmParams::deserialize(raw).map_err(ConfigError::LitellmParams)?;
        Ok(self
            .handler
            .handle_non_streaming(call.request_id, call.params, &params)
            .await?)
    }

    pub async fn handle_streaming(
        &mut self,
        call: &ConfigCall<'_>,
    ) -> Result<Vec<WxoStreamStep>, ConfigError<T::Error, K::Error>> {
        let raw = call
            .kwargs
            .get("litellm_params")
            .ok_or(ConfigError::MissingLitellmParams)?;
        let params = WxoLitellmParams::deserialize(raw).map_err(ConfigError::LitellmParams)?;
        Ok(self
            .handler
            .handle_streaming(
                call.request_id,
                call.params,
                &params,
                StreamPacing::default(),
            )
            .await?)
    }
}
