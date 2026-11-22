use betterlite_models::decisions::openai::DecisionResponse as OpenAiDecisionResponse;
use betterlite_models::decisions::systemone::DecisionsResponse as SystemOneResponse;
use betterlite_models::decisions::{HiddenParams, Json, ir};
use betterlite_openai::decisions::call::DecisionsCall;
use betterlite_openai::decisions::call::fields::RequestFormat;
use betterlite_openai::decisions::openai::response::ResponseConversionError;
use betterlite_openai::decisions::openai::response::to_openai::ir_to_openai_response;
use betterlite_openai::decisions::transformation::responses::to_systemone::ir_to_systemone_response;
use rust_decimal::Decimal;

use super::config::DecisionsSendConfig;
use crate::helpers::set_response_cost_in_hidden_params;

// DecisionsResponse | OpenAIDecisionResponse, chosen by the request format rather than the provider
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionsOutput {
    SystemOne(SystemOneResponse),
    OpenAi(OpenAiDecisionResponse),
}

// _format_response: convert through the request's shape, then set the hidden params on the result
pub(crate) fn format_response(
    config: &dyn DecisionsSendConfig,
    call: &DecisionsCall<'_>,
    response: &ir::DecisionResponse,
) -> Result<DecisionsOutput, ResponseConversionError> {
    let cost = config.provider_reported_cost(response);
    let output = match &call.request {
        RequestFormat::SystemOne(_) => {
            check_answers(response, call)?;
            let mut converted = ir_to_systemone_response(response, &call.ir_request);
            hidden_params(&mut converted.hidden_params, call, cost);
            DecisionsOutput::SystemOne(converted)
        }
        RequestFormat::OpenAi(_) => {
            let mut converted =
                ir_to_openai_response(response, &call.ir_request, &call.requested_model)?;
            hidden_params(&mut converted.hidden_params, call, cost);
            DecisionsOutput::OpenAi(converted)
        }
    };
    Ok(output)
}

// ir_to_systemone_response zips without strict=True, while LiteLLM raises when the counts differ
fn check_answers(
    response: &ir::DecisionResponse,
    call: &DecisionsCall<'_>,
) -> Result<(), ResponseConversionError> {
    let questions = call.ir_request.questions.len();
    if response.answers.len() == questions {
        return Ok(());
    }
    Err(ResponseConversionError::AnswerCount {
        answers: response.answers.len(),
        questions,
    })
}

// set_hidden_params merges these into the result; additional_headers is absent unless the config reports a cost
fn hidden_params(hidden: &mut HiddenParams, call: &DecisionsCall<'_>, cost: Option<Decimal>) {
    let qualified = format!("{}/{}", call.custom_llm_provider, call.model);
    hidden.insert(String::from("model"), Json::Text(qualified.clone()));
    hidden.insert(
        String::from("custom_llm_provider"),
        Json::Text(call.custom_llm_provider.clone()),
    );
    hidden.insert(
        String::from("provider_response_model"),
        Json::Text(qualified),
    );
    set_response_cost_in_hidden_params(hidden, cost);
}
