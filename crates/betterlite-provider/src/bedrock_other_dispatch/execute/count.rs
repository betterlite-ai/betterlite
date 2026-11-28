use betterlite_bedrock::count_call::counter::{BedrockTokenCounter, TokenCountResponse};
use betterlite_bedrock::count_call::handler::CountTokensHandler;

use super::super::call::CountOperation;

// No local tokenizer stands behind a failed or missing CountTokens answer: the arm reports None or the record
pub(super) async fn count<H>(operation: CountOperation<'_, H>) -> Option<TokenCountResponse>
where
    H: CountTokensHandler,
{
    let CountOperation { call, handler } = operation;
    let counter = BedrockTokenCounter { handler: handler? };
    counter.count_tokens(&call).await
}
