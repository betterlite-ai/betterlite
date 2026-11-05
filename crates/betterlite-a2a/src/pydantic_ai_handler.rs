use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::pydantic_ai_submit::error::SubmitError;
use crate::pydantic_ai_submit::{AgentCall, AgentTransport};
use crate::pydantic_ai_transformation::send::{Delivery, PydanticAITransformation};

mod tests;

#[derive(Debug, Error)]
pub enum HandlerError<E>
where
    E: StdError + 'static,
{
    #[error("api_base is required for Pydantic AI agents")]
    ApiBaseRequired,
    #[error(transparent)]
    Submit(#[from] SubmitError<E>),
}

pub struct HandlerRequest<'request> {
    pub request_id: &'request str,
    pub params: &'request AgentJson,
    pub api_base: Option<&'request str>,
    pub timeout: Duration,
    pub headers: &'request BTreeMap<String, String>,
}

pub struct PydanticAIHandler<T, S> {
    pub transformation: PydanticAITransformation<T, S>,
}

impl<T, S> PydanticAIHandler<T, S>
where
    T: AgentTransport,
    S: EventSource,
{
    pub async fn handle<D>(
        &mut self,
        request: &HandlerRequest<'_>,
        delivery: D,
    ) -> Result<D::Output, HandlerError<T::Error>>
    where
        D: Delivery,
    {
        Ok(self
            .transformation
            .send(&agent_call(request)?, delivery)
            .await?)
    }
}

fn agent_call<'request, E>(
    request: &HandlerRequest<'request>,
) -> Result<AgentCall<'request, AgentJson>, HandlerError<E>>
where
    E: StdError + 'static,
{
    let api_base = request.api_base.ok_or(HandlerError::ApiBaseRequired)?;
    Ok(AgentCall {
        api_base,
        request_id: request.request_id,
        params: request.params,
        timeout: request.timeout,
        headers: request.headers,
    })
}
