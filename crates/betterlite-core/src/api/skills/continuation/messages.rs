use std::fmt::Display;
use std::future::Future;

use serde_json::Value;

use crate::api::skills::hook::response::HookResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationRequest {
    pub model: String,
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
    pub max_tokens: u64,
}

pub trait MessagesContinuation {
    type Error: Display;

    fn create(
        &self,
        request: &ContinuationRequest,
    ) -> impl Future<Output = Result<Option<HookResponse>, Self::Error>> + Send;
}
