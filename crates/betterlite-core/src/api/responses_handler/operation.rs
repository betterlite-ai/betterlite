use std::error::Error as StdError;

use async_trait::async_trait;

use super::streams::{AsyncResponsesAnswer, SyncResponsesAnswer};
use crate::api::responses_transform::request::ResponsesRequest;

// litellm.responses(**responses_request): the creation call the sync entry point dispatches to, with the caller's per-call deps as its context.
pub trait ResponsesOperation: Send + Sync {
    type Context;
    type Error: StdError + Send + Sync + 'static;

    fn responses(
        &self,
        request: &ResponsesRequest,
        context: Self::Context,
    ) -> Result<SyncResponsesAnswer<Self::Error>, Self::Error>;
}

// litellm.aresponses(**responses_request): the awaited creation call the async entry point dispatches to.
#[async_trait]
pub trait AsyncResponsesOperation: Send + Sync {
    type Context: Send + 'static;
    type Error: StdError + Send + Sync + 'static;

    async fn aresponses(
        &self,
        request: &ResponsesRequest,
        context: Self::Context,
    ) -> Result<AsyncResponsesAnswer<Self::Error>, Self::Error>;
}
