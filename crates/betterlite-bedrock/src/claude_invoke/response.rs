use betterlite_anthropic::response::error::ResponseError;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{
    Logging, RawResponse, ResponseRequest, transform_response as anthropic_transform_response,
};
use betterlite_anthropic::usage::ReasoningTokenCounter;

// LiteLLM's AmazonAnthropicClaudeConfig.transform_response passes every argument to AnthropicConfig.
pub fn transform_response<Counter>(
    envelope: &mut ResponseEnvelope,
    raw: &RawResponse<'_>,
    request: &ResponseRequest<'_>,
    logging: Logging<'_>,
    counter: &Counter,
) -> Result<(), ResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    anthropic_transform_response(
        // the parent owns the usage, metadata, JSON-mode and error behavior this provider keeps
        envelope,
        raw,
        request,
        logging,
        counter,
    )
}
