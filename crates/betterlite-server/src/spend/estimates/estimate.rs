use std::collections::BTreeMap;
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
use betterlite_models::decisions::Json;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use betterlite_provider::pricing::image::error::ImagePolicyError;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::decode::SpendCalculateRequest;
use super::deployments::{find_deployment, matched_deployment};
use super::error::EstimateError;
use super::ports::EstimateRouterPort;
use crate::cost_service::{Call, CostService};

// calculate_spend prices a model with its messages, or the ModelResponse a call already produced.
pub async fn estimate(
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
    let matched = router.and_then(|router| {
        find_deployment(router.deployments(), router.model_group_alias(), requested)
    });
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
    let usage = usage(&response)?;
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
fn billing_input<'a>(
    model: Option<&'a str>,
    messages: &'a [Json],
    response: Option<&'a ResponseView<'a>>,
) -> CompletionBillingInput<'a> {
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

// ModelResponse(**request.completion_response) runs LiteLLM's chat decoder, whose usage defaults to zero counts.
fn decode_response(value: &Json) -> Result<ChatResponse, EstimateError> {
    let document = serde_json::to_value(value).map_err(|error| EstimateError::Failure {
        message: error.to_string(),
    })?;
    serde_json::from_value(document).map_err(|error| EstimateError::Failure {
        message: error.to_string(),
    })
}

fn usage(response: &ChatResponse) -> Result<Usage, EstimateError> {
    let prompt_tokens = count(response.usage.prompt_tokens, "prompt_tokens")?;
    let completion_tokens = count(response.usage.completion_tokens, "completion_tokens")?;
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        prompt_tokens_details: details(response.usage.prompt_tokens_details.as_ref()),
        completion_tokens_details: details(response.usage.completion_tokens_details.as_ref()),
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    })
}

fn count(value: u64, field: &'static str) -> Result<i64, EstimateError> {
    i64::try_from(value).map_err(|_conversion| EstimateError::Failure {
        message: format!("the response's {field} does not fit a signed 64-bit counter"),
    })
}

fn details(map: Option<&BTreeMap<String, Option<u64>>>) -> Option<IndexMap<String, Json>> {
    let map = map?;
    Some(
        map.iter()
            .filter_map(|(name, count)| {
                count.map(|count| (name.clone(), Json::Number(Number::from(count))))
            })
            .collect(),
    )
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
        .map_err(cost_error)
}

// An unmapped model is ModelNotMappedError; no resolvable provider is BadRequestError, which keeps its null type and param.
fn cost_error(error: CompletionCostError<ImagePolicyError>) -> EstimateError {
    if matches!(
        &error,
        CompletionCostError::Price(CallPriceError::Generic(TokenCostError::UnknownModel))
    ) {
        return EstimateError::ModelNotMapped {
            message: error.to_string(),
        };
    }
    if matches!(
        &error,
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
