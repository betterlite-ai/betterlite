use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_models::decisions::Json;

use super::{MessagesAgenticStreamInput, MessagesAgenticStreamResult};
use crate::api::messages_fake_stream::{MessagesFakeStream, MessagesFakeStreamError};
use crate::api::messages_sse_events::stream::MessagesProviderStream;

pub type AgenticHookFuture<FollowUp> = Pin<
    Box<
        dyn Future<Output = Result<MessagesAgenticStreamResult<FollowUp>, AgenticHookError>> + Send,
    >,
>;

// LiteLLM's _call_agentic_completion_hooks, injected so later provider interceptors plug in here
pub trait AgenticInterceptor: Send + Sync {
    type FollowUp: AgenticFollowUp + Send + 'static;

    fn intercept(
        &self,
        response: &Json,
        input: &MessagesAgenticStreamInput,
    ) -> AgenticHookFuture<Self::FollowUp>;
}

// a hooks answer that is a whole response reaches the client through the follow-up's from_response
pub trait AgenticFollowUp: MessagesProviderStream {
    fn from_response(response: &Json) -> Result<Self, MessagesFakeStreamError>
    where
        Self: Sized;
}

pub struct MessagesFakeFollowUp {
    frames: MessagesFakeStream,
}

#[async_trait]
impl MessagesProviderStream for MessagesFakeFollowUp {
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        Ok(self.frames.next())
    }
}

impl AgenticFollowUp for MessagesFakeFollowUp {
    fn from_response(response: &Json) -> Result<Self, MessagesFakeStreamError> {
        Ok(Self {
            frames: MessagesFakeStream::new(response)?,
        })
    }
}
