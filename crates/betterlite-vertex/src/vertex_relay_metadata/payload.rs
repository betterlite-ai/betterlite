use rust_decimal::Decimal;

use super::types::{
    CompletionCost, CompletionCostRequest, CompletionResponse, CustomLlmProvider, LoggingObject,
    ResponseLoggingKwargs,
};

pub struct ResponseLoggingPayloadInput<'input> {
    pub response: &'input mut CompletionResponse,
    pub model: &'input str,
    pub kwargs: ResponseLoggingKwargs,
    pub logging: &'input mut LoggingObject,
    pub custom_llm_provider: CustomLlmProvider,
    pub vertex_location: Option<&'input str>,
}

pub fn response_logging_payload(
    cost: &dyn CompletionCost,
    input: ResponseLoggingPayloadInput<'_>,
) -> ResponseLoggingKwargs {
    let ResponseLoggingPayloadInput {
        response,
        model,
        mut kwargs,
        logging,
        custom_llm_provider,
        vertex_location,
    } = input;
    let request = CompletionCostRequest {
        response,
        model,
        custom_llm_provider,
        vertex_location,
    };
    // LiteLLM lets a cost failure rise and the logging worker drops the row; betterlite records zero instead.
    let response_cost = cost.completion_cost(&request).unwrap_or_else(|| {
        tracing::warn!(
            model,
            provider = custom_llm_provider.as_str(),
            "vertex passthrough: response cost unavailable; recording 0 cost"
        );
        Decimal::ZERO
    });
    kwargs.response_cost = Some(response_cost);
    kwargs.model = Some(model.to_owned());
    response.id = Some(logging.litellm_call_id.clone());
    let logging_model = response
        .model
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(model)
        .to_owned();
    logging.model = Some(logging_model.clone());
    logging.model_call_details.model = Some(logging_model);
    logging.model_call_details.custom_llm_provider = Some(custom_llm_provider);
    kwargs
}
