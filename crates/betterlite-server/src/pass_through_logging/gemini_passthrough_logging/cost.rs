use betterlite_core::routes::CallType;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use betterlite_openai::videos::reply::VideoObject;
use betterlite_vertex::vertex_relay_metadata::types::{
    CompletionCost, CompletionCostRequest, CompletionResponse,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use super::reply;

// LiteLLM prices the response object it is about to log, so the cost service reads the logged body itself.
pub enum CostResponse<'response> {
    Completion(&'response Json),
    Video(&'response VideoObject),
}

pub struct CostRequest<'request> {
    pub response: CostResponse<'request>,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub call_type: CallType,
}

// B068's completion_cost, cut to the calls the passthrough leaves make; the proxy binds one implementation.
pub trait PassthroughCompletionCost: Send + Sync {
    fn completion_cost(&self, request: &CostRequest<'_>) -> Result<Decimal, PassthroughCostError>;
}

#[derive(Debug, Error)]
pub enum PassthroughCostError {
    #[error("the response cost of model {model} could not be resolved")]
    Unavailable { model: String },
    #[error("the response cost of model {model} could not be resolved: {source}")]
    Resolve {
        model: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

// B3638's factory prices through its own trait, so the Vertex leaf adapts the shared service onto it.
pub(crate) struct FactoryCost<'cost>(pub(crate) &'cost dyn PassthroughCompletionCost);

impl CompletionCost for FactoryCost<'_> {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Option<Decimal> {
        let body = completion_body(request.response);
        self.0
            .completion_cost(&CostRequest {
                response: CostResponse::Completion(&body),
                model: request.model,
                custom_llm_provider: request.custom_llm_provider.as_str(),
                call_type: CallType::Completion,
            })
            .ok()
    }
}

// The logging view of a B3638 completion: the fields a cost service reads back from a ModelResponse.
pub(crate) fn completion_body(response: &CompletionResponse) -> Json {
    let mut fields = IndexMap::new();
    if let Some(id) = &response.id {
        fields.insert("id".to_owned(), Json::Text(id.clone()));
    }
    if let Some(model) = &response.model {
        fields.insert("model".to_owned(), Json::Text(model.clone()));
    }
    if let Some(usage) = &response.usage {
        fields.insert("usage".to_owned(), reply::usage_json(&Usage::from(usage)));
    }
    Json::Object(fields)
}
