use std::convert::Infallible;
use std::time::SystemTime;

use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_core::cost::call_price::error::CallPriceError;
use betterlite_core::cost::completion::args::{
    ChargeInputs, CompletionArgs, ResponseParts, ToolInputs,
};
use betterlite_core::cost::completion::error::CompletionCostError;
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::prepare::{
    BillingTokenCounter, CompletionBillingInput, ResponseView, TypedResponse,
};
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::{ImageParts, LoggingView, TranscriptionDuration};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_core::routes::CallType;
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::decisions::Json;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use betterlite_provider::pricing::image::error::ImagePolicyError;
use rust_decimal::Decimal;

use super::decode::SpendCalculateRequest;
use super::deployments::{find_deployment, matched_deployment};
use super::error::EstimateError;
use super::ports::EstimateRouterPort;
use crate::cost_service::{Call, CostService};

// calculate_spend prices a model with its messages, or the ModelResponse a call already produced.
pub(crate) async fn estimate(
    service: &CostService<ProviderFormulas>,
    counter: &dyn BillingTokenCounter,
    router: Option<&dyn EstimateRouterPort>,
    request: &SpendCalculateRequest,
    instant: SystemTime,
) -> Result<Decimal, EstimateError> {
    let Some(requested) = request.model.as_deref() else {
        let Some(response) = &request.completion_response else {
            return Err(EstimateError::InputRequired);
        };
        return price_response(service, counter, response, instant).await;
    };
    let Some(messages) = &request.messages else {
        return Err(EstimateError::MessagesRequired);
    };
    let matched = router
        .and_then(|port| find_deployment(port.deployments(), port.model_group_alias(), requested));
    let (model, custom_cost_per_token) = match matched {
        Some(deployment) => {
            let fields = matched_deployment(deployment)?;
            (fields.model, fields.custom_cost_per_token)
        }
        // an unmatched model is handed to completion_cost as it came in, an empty string included
        None => (Some(requested), None),
    };
    let billing = billing_input(model, messages, None);
    price(
        service,
        counter,
        billing,
        charge_inputs(custom_cost_per_token),
        instant,
    )
    .await
}

async fn price_response(
    service: &CostService<ProviderFormulas>,
    counter: &dyn BillingTokenCounter,
    value: &Json,
    instant: SystemTime,
) -> Result<Decimal, EstimateError> {
    let response = decode_response(value)?;
    let usage = response_usage(value)?;
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: Some(response.model.as_str()),
        usage: BillingUsage::Chat(&usage),
        service_tier: None,
        hidden_params: None,
    });
    let billing = billing_input(None, &[], Some(&view));
    price(service, counter, billing, charge_inputs(None), instant).await
}

// Both paths bill one completion call and leave every other option unset; they differ only in what they carry.
const fn billing_input<'input>(
    model: Option<&'input str>,
    messages: &'input [Json],
    response: Option<&'input ResponseView<'input>>,
) -> CompletionBillingInput<'input> {
    CompletionBillingInput {
        model,
        prompt: "",
        messages,
        completion: "",
        call_type: Some(CallType::Completion),
        custom_llm_provider: None,
        region_name: None,
        router_model_id: None,
        custom_pricing: None,
        base_model: None,
        optional_params: None,
        service_tier: None,
        response,
    }
}

// ModelResponse(**request.completion_response) validates the body with LiteLLM's chat decoder before any price is read.
fn decode_response(value: &Json) -> Result<ChatResponse, EstimateError> {
    let document = serde_json::to_value(value).map_err(|error| EstimateError::Failure {
        message: error.to_string(),
    })?;
    serde_json::from_value(document).map_err(|error| EstimateError::Failure {
        message: error.to_string(),
    })
}

// ModelResponse(**body) hands the body's usage object to LiteLLM's Usage constructor, which keeps the provider's
// top-level cache counters as prompt details; a response without usage prices as zero counts.
fn response_usage(value: &Json) -> Result<Usage, EstimateError> {
    let Json::Object(fields) = value else {
        return Err(EstimateError::Failure {
            message: "the completion response is not an object".to_owned(),
        });
    };
    match fields.get("usage") {
        Some(Json::Object(mapping)) => {
            usage_from_mapping(mapping).map_err(|error| EstimateError::Failure {
                message: error.to_string(),
            })
        }
        None | Some(Json::Null) => Ok(Usage::default()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(EstimateError::Failure {
                message: "the completion response's usage must be an object".to_owned(),
            })
        }
    }
}

async fn price(
    service: &CostService<ProviderFormulas>,
    counter: &dyn BillingTokenCounter,
    billing: CompletionBillingInput<'_>,
    charge: ChargeInputs<'static>,
    instant: SystemTime,
) -> Result<Decimal, EstimateError> {
    let args = CompletionArgs { billing, charge };
    let call = Call {
        args: &args,
        counter,
    };
    let mut clock = move || instant;
    service
        .completion_cost(&call, &mut clock, None::<&mut NoBreakdown>)
        .await
        .map_err(|error| cost_error(&error))
}

// An unmapped model is ModelNotMappedError; no resolvable provider is BadRequestError, which keeps its null type and param.
fn cost_error(error: &CompletionCostError<ImagePolicyError>) -> EstimateError {
    if matches!(
        error,
        CompletionCostError::Price(CallPriceError::Generic(TokenCostError::UnknownModel))
    ) {
        return EstimateError::ModelNotMapped {
            message: error.to_string(),
        };
    }
    if matches!(
        error,
        CompletionCostError::Price(CallPriceError::Resolution { .. })
    ) {
        return EstimateError::ProviderMissing {
            message: error.to_string(),
        };
    }
    EstimateError::Failure {
        message: error.to_string(),
    }
}

// The route never records a breakdown: only the total cost is answered.
struct NoBreakdown;

impl CostBreakdownTarget for NoBreakdown {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, _input: &CostBreakdownInput) -> Result<(), Infallible> {
        Ok(())
    }
}

fn charge_inputs(custom_cost_per_token: Option<CustomCostPerToken>) -> ChargeInputs<'static> {
    ChargeInputs {
        passthrough_image: false,
        total_time: None,
        data_residency: None,
        vertex_location: None,
        custom_cost_per_token,
        custom_cost_per_second: None,
        logging: LoggingView {
            model: None,
            params: None,
            timing: None,
        },
        response: response_parts(),
        ocr_billing: None,
        vector_response: None,
        search_hidden_params: None,
        server_tool_projection: None,
        tools: tool_inputs(),
    }
}

fn response_parts() -> ResponseParts<'static> {
    ResponseParts {
        cost_response: None,
        image: ImageParts {
            response: ImageResponseView::Other,
            quality: None,
            image_count: None,
            size: None,
        },
        video_usage: None,
        transcription: TranscriptionDuration::default(),
        rerank: None,
        protocol: None,
        replicate: ReplicateStamps::default(),
        model_response: None,
    }
}

fn tool_inputs() -> ToolInputs {
    ToolInputs {
        response: ToolBillingResponse::Other(OtherView { usage: None }),
        params: BuiltInToolParams::default(),
        google_maps_requests: None,
    }
}
