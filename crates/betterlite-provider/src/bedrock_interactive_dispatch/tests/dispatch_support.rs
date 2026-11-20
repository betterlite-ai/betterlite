#![cfg(test)]

// Stands in for the realtime transport on the four leaves that open no session
#[derive(Debug, thiserror::Error)]
#[error("this operation opens no realtime session")]
pub(crate) struct NoBridgeTransport;

// Every case hands its leaf future to the production dispatcher, so the wiring and the leaf run together
macro_rules! dispatched {
    ($variant:ident, $future:expr) => {
        match block_on(crate::bedrock_interactive_dispatch::dispatch(
            crate::bedrock_interactive_dispatch::BedrockInteractiveOperation::<
                crate::bedrock_interactive_dispatch::tests::dispatch_support::NoBridgeTransport,
            >::$variant(Box::pin($future)),
        )) {
            Ok(crate::bedrock_interactive_dispatch::BedrockInteractiveResult::$variant(value)) => {
                Ok(value)
            }
            Err(crate::bedrock_interactive_dispatch::BedrockInteractiveError::$variant(error)) => {
                Err(error)
            }
            other => panic!("unexpected dispatch result: {other:?}"),
        }
    };
}

// The realtime arm names the exact transport failure the bridge carries, so no downcast is left
macro_rules! dispatched_realtime {
    ($transport:ty, $future:expr) => {
        match block_on(crate::bedrock_interactive_dispatch::dispatch(
            crate::bedrock_interactive_dispatch::BedrockInteractiveOperation::<$transport>::RealtimeBridge(
                $future,
            ),
        )) {
            Ok(crate::bedrock_interactive_dispatch::BedrockInteractiveResult::RealtimeBridge(
                value,
            )) => Ok(value),
            Err(crate::bedrock_interactive_dispatch::BedrockInteractiveError::RealtimeBridge(
                error,
            )) => Err(error),
            other => panic!("unexpected dispatch result: {other:?}"),
        }
    };
}

pub(crate) use dispatched;
pub(crate) use dispatched_realtime;
