use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_submit::error::SubmitError;
use crate::pydantic_ai_submit::{AgentCall, AgentTransport};
use crate::pydantic_ai_transformation::fake_streaming::{SimulatedEvent, StreamPacing};
use crate::pydantic_ai_transformation::send::PydanticAITransformation;
use crate::pydantic_ai_transformation::transform::ResponseMessage;

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

pub struct HandlerRequest<'request, P> {
    pub request_id: &'request str,
    pub params: &'request P,
    pub api_base: Option<&'request str>,
    pub timeout: Duration,
    pub headers: &'request BTreeMap<String, String>,
}

pub struct PydanticAIHandler<T> {
    pub transformation: PydanticAITransformation<T>,
}

impl<T> PydanticAIHandler<T>
where
    T: AgentTransport,
{
    pub async fn handle_non_streaming<P, S>(
        &self,
        request: &HandlerRequest<'_, P>,
        source: &mut S,
    ) -> Result<Event<ResponseMessage>, HandlerError<T::Error>>
    where
        P: Serialize + Sync,
        S: EventSource,
    {
        let call = agent_call(request)?;
        Ok(self
            .transformation
            .send_non_streaming_request(&call, source)
            .await?)
    }

    // The simulated events carry their own delays, so the caller paces the replay
    pub async fn handle_streaming<P, S>(
        &self,
        request: &HandlerRequest<'_, P>,
        pacing: StreamPacing,
        source: &mut S,
    ) -> Result<Vec<SimulatedEvent>, HandlerError<T::Error>>
    where
        P: Serialize + Sync,
        S: EventSource,
    {
        let call = agent_call(request)?;
        Ok(self
            .transformation
            .send_streaming_request(&call, pacing, source)
            .await?)
    }
}

fn agent_call<'request, P, E>(
    request: &HandlerRequest<'request, P>,
) -> Result<AgentCall<'request, P>, HandlerError<E>>
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
