use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use betterlite_core::agents::json::AgentJson;
use futures_core::Stream;

use super::events::StreamEvent;
use super::wire::{MessageSendParams, SendMessageResponse};

pub trait BaseA2AProviderConfig: Send + Sync {
    type Error: Error + Send + Sync;
    type EventStream: Stream<Item = Result<StreamEvent, Self::Error>> + Send;

    fn handle_non_streaming(
        &self,
        request_id: &str,
        params: &MessageSendParams,
        api_base: Option<&str>,
        kwargs: &BTreeMap<String, AgentJson>,
    ) -> impl Future<Output = Result<SendMessageResponse, Self::Error>> + Send;

    fn handle_streaming(
        &self,
        request_id: &str,
        params: &MessageSendParams,
        api_base: Option<&str>,
        kwargs: &BTreeMap<String, AgentJson>,
    ) -> impl Future<Output = Result<Self::EventStream, Self::Error>> + Send;
}

mod tests;
