use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_core::cost::completion::args::{
    ChargeInputs, CompletionArgs, ResponseParts, ToolInputs,
};
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::prepare::{
    BillingTokenCounter, CompletionBillingInput, ResponseView, TypedResponse,
};
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::{ImageParts, LoggingView, TranscriptionDuration};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_core::routes::CallType;
use betterlite_logging::fields::usage::Usage;
use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::decisions::Json;
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use betterlite_provider::pricing::grounding::ProviderFormulas;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use serde_json::Number;

use super::deployments::{DeploymentSet, ResolvedCostModel, unresolvable};
use super::error::EstimateError;
use super::lines::{Capture, CostLines};
use super::response::{reported_provider, response_for};
use crate::cost_service::{Call, CostService};
use crate::proxy_request_contracts::cost_estimate::{CostEstimateRequest, CostEstimateResponse};

pub(crate) async fn estimate(
    service: &CostService<ProviderFormulas>,
    deployments: Option<&DeploymentSet>,
    request: &CostEstimateRequest,
    instant: SystemTime,
) -> Result<CostEstimateResponse, EstimateError> {
    let resolved = deployments.map_or_else(
        || unresolvable(&request.model),
        |set| set.resolve(service, &request.model),
    );
    let usage = synthetic_usage(request);
    let response = typed_response(&resolved, &usage);
    let args = completion_args(&resolved, &response);
    let (lines, rates) = called_lines(service, &args, instant, request, &resolved).await?;
    Ok(response_for(
        request,
        &lines,
        rates,
        reported_provider(service, &resolved),
    ))
}

// The mock response carries the caller's model and the quote's synthetic usage, nothing else.
fn typed_response<'quote>(
    resolved: &'quote ResolvedCostModel,
    usage: &'quote Usage,
) -> ResponseView<'quote> {
    ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: Some(resolved.model.as_str()),
        usage: BillingUsage::Chat(usage),
        service_tier: None,
        hidden_params: None,
    })
}

async fn called_lines(
    service: &CostService<ProviderFormulas>,
    args: &CompletionArgs<'_>,
    instant: SystemTime,
    request: &CostEstimateRequest,
    resolved: &ResolvedCostModel,
) -> Result<(CostLines, Option<BilledTokenRates>), EstimateError> {
    let counters = NeverCounts;
    let call = Call {
        args,
        counter: &counters,
    };
    let mut clock = move || instant;
    let mut capture = Capture::default();
    let total = service
        .completion_cost(&call, &mut clock, Some(&mut capture))
        .await
        .map_err(|error| EstimateError::Calculation {
            message: format!(
                "Could not calculate cost for model '{}' (resolved to '{}'): {error}",
                request.model, resolved.model
            ),
        })?;
    Ok(capture.finish(total))
}

// The usage _usage_for_estimate builds: cache counts are details within the prompt, the reasoning split is within the completion.
fn synthetic_usage(request: &CostEstimateRequest) -> Usage {
    Usage {
        prompt_tokens: request.input_tokens,
        completion_tokens: request.output_tokens,
        total_tokens: request.input_tokens.saturating_add(request.output_tokens),
        prompt_tokens_details: prompt_details(request),
        completion_tokens_details: completion_details(request),
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

// PromptTokensDetailsWrapper is built only when any cache token is counted.
fn prompt_details(request: &CostEstimateRequest) -> Option<IndexMap<String, Json>> {
    let cache_tokens = request
        .cache_read_input_tokens
        .saturating_add(request.cache_creation_input_tokens);
    if cache_tokens == 0 {
        return None;
    }
    let mut details = IndexMap::new();
    details.insert(
        "cached_tokens".to_owned(),
        Json::Number(Number::from(request.cache_read_input_tokens)),
    );
    details.insert(
        "cache_creation_tokens".to_owned(),
        Json::Number(Number::from(request.cache_creation_input_tokens)),
    );
    Some(details)
}

// Usage.__init__ splits text out of the completion only when reasoning tokens were reported.
fn completion_details(request: &CostEstimateRequest) -> Option<IndexMap<String, Json>> {
    if request.reasoning_tokens == 0 {
        return None;
    }
    let mut details = IndexMap::new();
    details.insert(
        "reasoning_tokens".to_owned(),
        Json::Number(Number::from(request.reasoning_tokens)),
    );
    details.insert(
        "text_tokens".to_owned(),
        Json::Number(Number::from(
            request
                .output_tokens
                .saturating_sub(request.reasoning_tokens),
        )),
    );
    Some(details)
}

// completion_cost is never asked to count anything: the quote's only tokens are the ones the request declares.
pub(super) struct NeverCounts;

#[async_trait]
impl BillingTokenCounter for NeverCounts {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _model: &str, _text: &str) -> Result<usize, CountError> {
        Ok(0)
    }
}

fn completion_args<'quote>(
    resolved: &'quote ResolvedCostModel,
    response: &'quote ResponseView<'quote>,
) -> CompletionArgs<'quote> {
    CompletionArgs {
        billing: billing_input(resolved, response),
        charge: charge_inputs(resolved),
    }
}

fn billing_input<'quote>(
    resolved: &'quote ResolvedCostModel,
    response: &'quote ResponseView<'quote>,
) -> CompletionBillingInput<'quote> {
    CompletionBillingInput {
        model: Some(resolved.model.as_str()),
        prompt: "",
        messages: &[],
        completion: "",
        call_type: Some(CallType::Completion),
        custom_llm_provider: resolved.provider.as_deref(),
        region_name: None,
        router_model_id: None,
        custom_pricing: None,
        base_model: None,
        optional_params: None,
        service_tier: None,
        response: Some(response),
    }
}

fn charge_inputs(resolved: &ResolvedCostModel) -> ChargeInputs<'_> {
    ChargeInputs {
        passthrough_image: false,
        total_time: None,
        data_residency: None,
        vertex_location: None,
        custom_cost_per_token: resolved.custom_cost_per_token,
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
