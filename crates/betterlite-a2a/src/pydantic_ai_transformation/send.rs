use serde::{Deserialize as _, Serialize};

use super::fake_streaming::{SimulatedEvent, StreamPacing, fake_streaming_from_response};
use super::transform::{ResponseMessage, transform_to_a2a_response};
use super::wire::TaskResponse;
use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_submit::error::SubmitError;
use crate::pydantic_ai_submit::{AgentCall, AgentTransport, send_and_poll_raw};

mod tests;

pub struct PydanticAITransformation<T, S> {
    pub transport: T,
    pub source: S,
}

pub struct Polled<'polled, S> {
    pub task: &'polled TaskResponse,
    pub request_id: &'polled str,
    pub source: &'polled mut S,
}

// How the raw task leaves the transformation: projected to one message or replayed as a stream
pub trait Delivery: From<StreamPacing> {
    type Output;

    // LiteLLM refuses an empty api_base only on the streaming path
    const REFUSES_EMPTY_API_BASE: bool;

    fn deliver<S>(self, polled: Polled<'_, S>) -> Self::Output
    where
        S: EventSource;
}

pub struct ProjectedMessage;

impl From<StreamPacing> for ProjectedMessage {
    fn from(_pacing: StreamPacing) -> Self {
        Self
    }
}

impl Delivery for ProjectedMessage {
    type Output = Event<ResponseMessage>;
    const REFUSES_EMPTY_API_BASE: bool = false;

    fn deliver<S>(self, polled: Polled<'_, S>) -> Self::Output
    where
        S: EventSource,
    {
        transform_to_a2a_response(polled.task, polled.request_id, polled.source)
    }
}

// Streaming replays the raw task's history and artifacts, so nothing is projected first
pub struct ReplayedStream(pub StreamPacing);

impl From<StreamPacing> for ReplayedStream {
    fn from(pacing: StreamPacing) -> Self {
        Self(pacing)
    }
}

impl Delivery for ReplayedStream {
    type Output = Vec<SimulatedEvent>;
    const REFUSES_EMPTY_API_BASE: bool = true;

    fn deliver<S>(self, polled: Polled<'_, S>) -> Self::Output
    where
        S: EventSource,
    {
        fake_streaming_from_response(polled.task, polled.request_id, self.0, polled.source)
    }
}

impl<T, S> PydanticAITransformation<T, S>
where
    T: AgentTransport,
    S: EventSource,
{
    pub async fn send<P, D>(
        &mut self,
        call: &AgentCall<'_, P>,
        delivery: D,
    ) -> Result<D::Output, SubmitError<T::Error>>
    where
        P: Serialize + Sync,
        D: Delivery,
    {
        let raw = send_and_poll_raw(&self.transport, call).await?;
        let task = TaskResponse::deserialize(&raw).map_err(SubmitError::Response)?;
        let polled = Polled {
            task: &task,
            request_id: call.request_id,
            source: &mut self.source,
        };
        Ok(delivery.deliver(polled))
    }
}
