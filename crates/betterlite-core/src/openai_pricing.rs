pub mod embedding_cost;
pub mod get_provider_config;
pub mod image_cost;
pub mod responses_api;
pub mod route_predicates;

use std::error::Error;

use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::response_events::ResponsesReply;
use rust_decimal::Decimal;

use crate::routes::CallType;

pub enum CostedResponse<'request> {
    Embedding(&'request EmbeddingResponse),
    Responses(&'request ResponsesReply),
}

pub struct CompletionCostRequest<'request> {
    pub response: CostedResponse<'request>,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub call_type: CallType,
}

pub type CostError = Box<dyn Error + Send + Sync>;

// The async pricing pipeline cannot run inside a synchronous relay handler, so the caller binds a cost service that can
pub trait CompletionCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Result<Decimal, CostError>;
}
