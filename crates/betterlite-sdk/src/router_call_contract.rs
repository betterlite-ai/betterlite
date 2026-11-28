pub mod error;
pub mod executors;

mod tests;

use async_trait::async_trait;
use betterlite_core::api::passthrough_call::PassthroughReply;
use betterlite_models::decisions::{HiddenParams, Json};

use self::error::GenericError;
use crate::call_setup::RequestMap;

// LiteLLM merges {**data, **kwargs} into one dict, so a later key wins and the order stays as sent.
pub struct GenericRequest {
    pub call_type: String,
    pub args: RequestMap,
}

#[async_trait]
pub trait GenericCallExecutor: Send + Sync {
    async fn call(&self, request: GenericRequest) -> Result<GenericOutcome, GenericError>;
}

// What the callable returned, kept in the form LiteLLM's original_generic_function produced.
pub enum GenericOutcome {
    Json(Json),
    Bytes(Vec<u8>),
    Stream(Box<dyn GenericStream>),
    Http(PassthroughReply),
}

// The attributes FallbackAwareAnthropicMessagesStream reads off its source iterator.
#[async_trait]
pub trait GenericStream: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, GenericError>;
    async fn close(&mut self);
    fn hidden_params(&self) -> HiddenParams;
    fn has_buffered_provider_output(&self) -> bool;
    fn chunks(&self) -> Option<&[Json]>;
    fn messages(&self) -> Option<&[Json]>;
    fn model(&self) -> Option<&str>;
}
