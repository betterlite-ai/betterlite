use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;
use serde_with::{DurationSecondsWithFrac, serde_as};
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::pydantic_ai_handler::{HandlerError, HandlerRequest, PydanticAIHandler};
use crate::pydantic_ai_submit::{AgentTransport, DEFAULT_TIMEOUT};
use crate::pydantic_ai_transformation::send::Delivery;

mod tests;

#[derive(Debug, Error)]
pub enum ProviderError<E>
where
    E: StdError + 'static,
{
    #[error("the Pydantic AI provider arguments are invalid")]
    Kwargs(#[source] serde_json::Error),
    #[error(transparent)]
    Handler(#[from] HandlerError<E>),
}

pub struct ProviderCall<'call> {
    pub request_id: &'call str,
    pub params: &'call AgentJson,
    pub api_base: Option<&'call str>,
    pub kwargs: &'call BTreeMap<String, AgentJson>,
}

#[serde_as]
#[derive(Deserialize)]
struct ProviderKwargs {
    #[serde(default)]
    #[serde_as(as = "Option<DurationSecondsWithFrac<f64>>")]
    timeout: Option<Duration>,
    #[serde(default)]
    agent_extra_headers: Option<BTreeMap<String, String>>,
}

pub struct PydanticAIProviderConfig<T, S> {
    pub handler: PydanticAIHandler<T, S>,
}

impl<T, S> PydanticAIProviderConfig<T, S>
where
    T: AgentTransport,
    S: EventSource,
{
    pub async fn handle<D>(
        &mut self,
        call: &ProviderCall<'_>,
    ) -> Result<D::Output, ProviderError<T::Error>>
    where
        D: Delivery,
    {
        if D::REFUSES_EMPTY_API_BASE && call.api_base.is_none_or(str::is_empty) {
            return Err(ProviderError::Handler(HandlerError::ApiBaseRequired));
        }
        let fields = AgentJson::Object(call.kwargs.clone().into_iter().collect());
        let parsed = ProviderKwargs::deserialize(&fields).map_err(ProviderError::Kwargs)?;
        let delivery = D::from_kwargs(&fields).map_err(ProviderError::Kwargs)?;
        let headers = parsed.agent_extra_headers.unwrap_or_default();
        let request = HandlerRequest {
            request_id: call.request_id,
            params: call.params,
            api_base: call.api_base,
            timeout: parsed.timeout.unwrap_or(DEFAULT_TIMEOUT),
            headers: &headers,
        };
        Ok(self.handler.handle(&request, delivery).await?)
    }
}
