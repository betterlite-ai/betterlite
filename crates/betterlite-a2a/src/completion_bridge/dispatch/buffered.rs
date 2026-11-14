use std::future::Future;

use betterlite_core::agents::json::AgentJson;
use futures_util::TryFutureExt as _;

use super::{
    A2AProviderDispatch, BridgeError, Buffered, CompletionError, Mode, handle_non_streaming_direct,
};
use crate::completion_bridge::EventSource;
use crate::completion_bridge::completion::CompletionPort;
use crate::completion_bridge::params::BridgeCall;
use crate::provider_selector::A2AProviderConfig;

impl<'call, C, P> Mode<'call, C, P> for Buffered
where
    C: CompletionPort,
    P: A2AProviderDispatch,
{
    type Output = Result<AgentJson, BridgeError<C::Error, P::Error>>;

    fn provider(
        providers: &'call P,
        config: A2AProviderConfig,
        call: &'call BridgeCall<'_>,
    ) -> impl Future<Output = Self::Output> + Send + 'call {
        providers
            .handle_non_streaming(config, call)
            .map_err(BridgeError::Provider)
    }

    fn direct<S>(
        call: &'call BridgeCall<'_>,
        completion: &'call C,
        source: &'call mut S,
    ) -> impl Future<Output = Self::Output> + Send + 'call
    where
        S: EventSource + Send + 'call,
    {
        handle_non_streaming_direct(call, completion, source).map_err(BridgeError::Completion)
    }

    fn failed(error: CompletionError<C::Error>) -> Self::Output {
        Err(BridgeError::Completion(error))
    }
}
