use std::error::Error;
use std::future::{Future, ready};

use futures_util::StreamExt as _;
use futures_util::stream::iter;

use super::{
    A2AProviderDispatch, BridgeError, BridgeEvents, CompletionError, Mode, Streamed,
    handle_streaming_direct,
};
use crate::completion_bridge::EventSource;
use crate::completion_bridge::completion::CompletionPort;
use crate::completion_bridge::params::BridgeCall;
use crate::provider_selector::A2AProviderConfig;

impl<'call, C, P> Mode<'call, C, P> for Streamed
where
    C: CompletionPort,
    P: A2AProviderDispatch,
{
    type Output = BridgeEvents<'call, BridgeError<C::Error, P::Error>>;

    fn provider(
        providers: &'call P,
        config: A2AProviderConfig,
        call: &'call BridgeCall<'_>,
    ) -> impl Future<Output = Self::Output> + Send + 'call {
        provider_events::<C::Error, P>(providers, config, call)
    }

    fn direct<S>(
        call: &'call BridgeCall<'_>,
        completion: &'call C,
        source: &'call mut S,
    ) -> impl Future<Output = Self::Output> + Send + 'call
    where
        S: EventSource + Send + 'call,
    {
        let events = handle_streaming_direct(call, completion, source)
            .map(|event| event.map_err(BridgeError::Completion))
            .boxed();
        ready(events)
    }

    fn failed(error: CompletionError<C::Error>) -> Self::Output {
        iter(Some(Err(BridgeError::Completion(error)))).boxed()
    }
}

async fn provider_events<'call, E, P>(
    providers: &'call P,
    config: A2AProviderConfig,
    call: &'call BridgeCall<'_>,
) -> BridgeEvents<'call, BridgeError<E, P::Error>>
where
    E: Error + Send + Sync + 'static,
    P: A2AProviderDispatch,
{
    match providers.handle_streaming(config, call).await {
        Ok(events) => events
            .map(|event| event.map_err(BridgeError::Provider))
            .boxed(),
        Err(error) => iter(Some(Err(BridgeError::Provider(error)))).boxed(),
    }
}
