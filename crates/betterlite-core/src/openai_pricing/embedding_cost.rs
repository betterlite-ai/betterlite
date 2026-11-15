use betterlite_openai::embedding::EmbeddingResponse;
use rust_decimal::Decimal;

use crate::openai_pricing::{CompletionCost, CompletionCostRequest, CostedResponse};
use crate::routes::CallType;

mod tests;

#[must_use]
pub fn embedding_cost(
    embedding: &EmbeddingResponse,
    model: &str,
    custom_llm_provider: &str,
    cost: &dyn CompletionCost,
) -> Decimal {
    let request = CompletionCostRequest {
        response: CostedResponse::Embedding(embedding),
        model,
        custom_llm_provider,
        call_type: CallType::Aembedding,
    };
    match cost.completion_cost(&request) {
        Ok(charge) => charge,
        Err(error) => {
            tracing::warn!(
                model,
                error = %error,
                "Error calculating embeddings cost for model {model}, logging spend with cost 0"
            );
            Decimal::ZERO
        }
    }
}
