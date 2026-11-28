use betterlite_bedrock::count_call::counter::{BedrockTokenCounter, TokenCountResponse};
use betterlite_bedrock::count_call::handler::CountTokensHandler;

use super::call::CountOperation;

// No local tokenizer stands behind a failed or missing CountTokens answer: the arm reports None or the record
pub(super) async fn count<H>(operation: CountOperation<'_, H>) -> Option<TokenCountResponse>
where
    H: CountTokensHandler,
{
    let CountOperation { call, handler } = operation;
    let handler = handler?;
    let counter = BedrockTokenCounter { handler };
    counter.count_tokens(&call).await
}
