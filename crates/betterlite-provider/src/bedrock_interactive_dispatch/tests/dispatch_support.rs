// Every case hands its leaf future to the production dispatcher, so the wiring and the leaf run together
macro_rules! dispatched {
    ($variant:ident, $future:expr) => {
        match block_on(crate::bedrock_interactive_dispatch::dispatch(
            crate::bedrock_interactive_dispatch::BedrockInteractiveOperation::$variant(Box::pin(
                $future,
            )),
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

pub(crate) use dispatched;
