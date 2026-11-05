use serde::{Deserialize as _, Serialize};

use super::fake_streaming::{SimulatedEvent, StreamPacing, fake_streaming_from_response};
use super::transform::{ResponseMessage, transform_to_a2a_response};
use super::wire::TaskResponse;
use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_submit::error::SubmitError;
use crate::pydantic_ai_submit::{AgentCall, AgentTransport, send_and_poll_raw};

mod tests;

pub struct PydanticAITransformation<T> {
    pub transport: T,
}

impl<T> PydanticAITransformation<T>
where
    T: AgentTransport,
{
    pub async fn send_non_streaming_request<P, S>(
        &self,
        call: &AgentCall<'_, P>,
        source: &mut S,
    ) -> Result<Event<ResponseMessage>, SubmitError<T::Error>>
    where
        P: Serialize + Sync,
        S: EventSource,
    {
        let raw = send_and_poll_raw(&self.transport, call).await?;
        let task = TaskResponse::deserialize(&raw).map_err(SubmitError::Response)?;
        Ok(transform_to_a2a_response(&task, call.request_id, source))
    }

    // Streaming replays the raw task's history and artifacts, so nothing is projected first
    pub async fn send_streaming_request<P, S>(
        &self,
        call: &AgentCall<'_, P>,
        pacing: StreamPacing,
        source: &mut S,
    ) -> Result<Vec<SimulatedEvent>, SubmitError<T::Error>>
    where
        P: Serialize + Sync,
        S: EventSource,
    {
        let raw = send_and_poll_raw(&self.transport, call).await?;
        let task = TaskResponse::deserialize(&raw).map_err(SubmitError::Response)?;
        Ok(fake_streaming_from_response(
            &task,
            call.request_id,
            pacing,
            source,
        ))
    }
}
