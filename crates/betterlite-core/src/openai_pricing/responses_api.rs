use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::response_events::{RawResponse, ResponsesReply, transform_response};
use rust_decimal::Decimal;

use crate::openai_pricing::cost_error::CostError;
use crate::openai_pricing::{CompletionCost, CompletionCostRequest, CostedResponse};
use crate::routes::CallType;

mod tests;

pub struct ResponsesCostRequest<'request> {
    pub reply: &'request RawResponse<'request>,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub now: i64,
    pub state: &'request mut LoggingState,
    pub env: &'request PrePostEnv<'request>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResponsesApiError {
    #[error("the Responses reply could not be transformed: {0}")]
    Transform(#[from] OpenAiError),
    #[error("the Responses cost could not be calculated: {source}")]
    Cost {
        #[source]
        source: CostError,
    },
}

pub fn responses_with_cost(
    request: ResponsesCostRequest<'_>,
    cost: &dyn CompletionCost,
) -> Result<(ResponsesReply, Decimal), ResponsesApiError> {
    let ResponsesCostRequest {
        reply,
        model,
        custom_llm_provider,
        now,
        state,
        env,
    } = request;
    let transformed = transform_response(reply, now, state, env)?;
    let charge = cost
        .completion_cost(&CompletionCostRequest {
            response: CostedResponse::Responses(&transformed),
            model,
            custom_llm_provider,
            call_type: CallType::Responses,
        })
        .map_err(|source| ResponsesApiError::Cost { source })?;
    Ok((transformed, charge))
}
