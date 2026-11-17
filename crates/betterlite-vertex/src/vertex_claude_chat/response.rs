use betterlite_anthropic::response::error::ResponseError;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{
    Logging, RawResponse, ResponseRequest, transform_response as parent_transform_response,
};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;

pub struct TransformCall<'call, Counter> {
    pub model: &'call str,
    pub envelope: &'call mut ResponseEnvelope,
    pub raw: &'call RawResponse<'call>,
    pub request: &'call ResponseRequest<'call>,
    pub state: &'call mut LoggingState,
    pub env: &'call PrePostEnv<'call>,
    pub counter: &'call Counter,
}

// The parent fills the envelope from the body, then Vertex names the model the caller asked for.
pub fn transform_response<Counter>(
    call: TransformCall<'_, Counter>,
) -> Result<(), ResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let TransformCall {
        model,
        envelope,
        raw,
        request,
        state,
        env,
        counter,
    } = call;
    parent_transform_response(envelope, raw, request, Logging { state, env }, counter)?;
    envelope.model = Some(model.to_owned());
    Ok(())
}
