use async_trait::async_trait;
use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::cost::context::{IdentityRequest, ProviderInference, billing_model_name};
use crate::cost::realtime::RealtimeBilling;
use crate::cost::realtime::responses_ws::split_by_service_tier;
use crate::cost::usage::provider_cost::without_provider_stated_cost;
use crate::cost::usage::tiers::{
    normalize_service_tier, resolve_billable_service_tier, served_service_tier,
};
use crate::cost::usage::{BillingUsage, ResponseKind, get_usage_object, infer_call_type};
use crate::routes::CallType;

mod attempt;
pub mod error;
mod tests;
mod usage_counts;
mod view;

use attempt::AttemptState;
use error::PrepareError;

pub struct CompletionBillingInput<'request> {
    pub model: Option<&'request str>,
    pub prompt: &'request str,
    pub messages: &'request [Json],
    pub completion: &'request str,
    pub call_type: Option<CallType>,
    pub custom_llm_provider: Option<&'request str>,
    pub region_name: Option<&'request str>,
    pub router_model_id: Option<&'request str>,
    pub custom_pricing: Option<bool>,
    pub base_model: Option<&'request str>,
    pub optional_params: Option<&'request IndexMap<String, Json>>,
    pub service_tier: Option<&'request str>,
    pub response: Option<&'request ResponseView<'request>>,
}

pub enum ResponseView<'response> {
    Realtime {
        record: &'response RealtimeBilling,
        // the endpoint owner projects the session's aggregate usage into the logging Usage here
        usage: &'response Usage,
    },
    Typed(TypedResponse<'response>),
    Dictionary(DictionaryResponse<'response>),
    Other,
}

pub struct TypedResponse<'response> {
    pub kind: ResponseKind,
    pub model: Option<&'response str>,
    pub usage: BillingUsage<'response>,
    pub service_tier: Option<&'response str>,
    pub hidden_params: Option<&'response IndexMap<String, Json>>,
}

// A dictionary response exposes only its keys, so it never carries hidden provider attributes
pub struct DictionaryResponse<'response> {
    pub model: Option<&'response str>,
    pub usage: BillingUsage<'response>,
    pub service_tier: Option<&'response str>,
}

pub enum PreparedCompletion {
    WebsocketPartitions(Vec<RealtimeBilling>),
    Attempts(Box<PreparedBilling>),
}

pub struct PreparedBilling {
    pub call_type: CallType,
    pub service_tier: Option<String>,
    pub selected_model: Option<String>,
    pub explicit_pricing: bool,
    // the usage object cost_per_token receives, with a provider-stated cost cleared under custom pricing
    pub cost_usage: Option<Usage>,
    // an unknown typed usage is replaced on the response by this normalized usage
    pub replaced_usage: Option<Usage>,
    pub attempts: Vec<PreparedBillingAttempt>,
}

pub struct PreparedBillingAttempt {
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub region_name: Option<String>,
    pub outcome: AttemptOutcome,
}

pub enum AttemptOutcome {
    Ready(AttemptBilling),
    Failed(PrepareError),
}

pub struct AttemptBilling {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    // B1689 fills these four before the ordinary token stage runs
    pub prompt_characters: Option<u64>,
    pub completion_characters: Option<u64>,
    pub transcription_duration: Decimal,
    pub rerank_units: Option<RerankUnits>,
}

pub struct RerankUnits {
    pub search_units: Option<i64>,
    pub total_tokens: Option<i64>,
}

#[async_trait]
pub trait BillingTokenCounter: Send + Sync {
    async fn count_messages(&self, model: &str, messages: &[Json]) -> Result<usize, CountError>;

    async fn count_text(&self, model: &str, text: &str) -> Result<usize, CountError>;
}

pub async fn prepare<Inference>(
    input: &CompletionBillingInput<'_>,
    lookup: &(dyn PriceLookup + Sync),
    inference: &Inference,
    counter: &dyn BillingTokenCounter,
) -> Result<PreparedCompletion, PrepareError>
where
    Inference: ProviderInference + Sync,
{
    let kind = input
        .response
        .map_or(ResponseKind::Other, ResponseView::kind);
    let call_type = infer_call_type(input.call_type, kind).unwrap_or(CallType::Completion);
    if let Some(parts) = websocket_partitions(input, call_type)? {
        return Ok(PreparedCompletion::WebsocketPartitions(parts));
    }
    let request_model = rewritten_model(input, call_type);
    let (cost_usage, service_tier) = priced_inputs(input)?;
    let explicit_pricing = input.custom_pricing == Some(true) || input.base_model.is_some();
    let cost_response = input.response.and_then(ResponseView::cost_response);
    let identity = IdentityRequest {
        model: request_model,
        response: cost_response.as_ref(),
        base_model: input.base_model,
        custom_pricing: input.custom_pricing,
        custom_llm_provider: input.custom_llm_provider,
        router_model_id: input.router_model_id,
        region_name: input.region_name,
    };
    let selected_model = billing_model_name(&identity, lookup, inference);
    let candidates = candidate_models(input.response, request_model, selected_model.as_deref());
    let mut state = AttemptState {
        custom_llm_provider: input.custom_llm_provider.map(str::to_owned),
        region_name: input.region_name.map(str::to_owned),
        installed_usage: None,
    };
    let attempts = candidate_attempts(input, &candidates, &mut state, counter).await;
    Ok(PreparedCompletion::Attempts(Box::new(PreparedBilling {
        call_type,
        service_tier,
        selected_model,
        explicit_pricing,
        cost_usage,
        replaced_usage: state.installed_usage,
        attempts,
    })))
}

fn rewritten_model<'request>(
    input: &CompletionBillingInput<'request>,
    call_type: CallType,
) -> Option<&'request str> {
    // Azure expects an empty model name for dall-e-2, and only the prompt-less image calls qualify
    let azure_image = matches!(
        call_type,
        CallType::ImageGeneration | CallType::AimageGeneration
    ) && input.model == Some("")
        && input.custom_llm_provider == Some("azure");
    if azure_image {
        return Some("dall-e-2");
    }
    input.model
}

fn resolved_service_tier(
    input: &CompletionBillingInput<'_>,
    usage: Option<&Usage>,
) -> Option<String> {
    if let Some(explicit) = normalize_service_tier(input.service_tier) {
        return Some(explicit.to_owned());
    }
    let requested = input
        .optional_params
        .and_then(|params| params.get("service_tier"))
        .and_then(|value| match value {
            Json::Text(text) => Some(text.as_str()),
            // a non-string request tier is not a billable tier, so the response decides
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        });
    let served = served_service_tier(
        input.response.and_then(ResponseView::service_tier),
        input.response.and_then(ResponseView::hidden_params),
        usage,
    );
    resolve_billable_service_tier(requested, served.as_deref())
}

fn websocket_partitions(
    input: &CompletionBillingInput<'_>,
    call_type: CallType,
) -> Result<Option<Vec<RealtimeBilling>>, PrepareError> {
    if call_type != CallType::PrivateAresponsesWebsocket {
        return Ok(None);
    }
    let Some(ResponseView::Realtime { record, .. }) = input.response else {
        return Ok(None);
    };
    split_by_service_tier(record).map_err(PrepareError::from)
}

fn priced_inputs(
    input: &CompletionBillingInput<'_>,
) -> Result<(Option<Usage>, Option<String>), PrepareError> {
    let mut cost_usage = match input.response {
        Some(response) => get_usage_object(&response.billing())?,
        None => None,
    };
    if input.custom_pricing == Some(true)
        && let Some(usage) = cost_usage.as_ref()
    {
        cost_usage = Some(without_provider_stated_cost(usage).into_owned());
    }
    let service_tier = resolved_service_tier(input, cost_usage.as_ref());
    Ok((cost_usage, service_tier))
}

async fn candidate_attempts(
    input: &CompletionBillingInput<'_>,
    candidates: &[Option<String>],
    state: &mut AttemptState,
    counter: &dyn BillingTokenCounter,
) -> Vec<PreparedBillingAttempt> {
    let mut attempts = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        attempts.push(attempt::prepare_attempt(input, candidate.as_deref(), state, counter).await);
    }
    attempts
}

fn candidate_models(
    response: Option<&ResponseView<'_>>,
    request_model: Option<&str>,
    selected_model: Option<&str>,
) -> Vec<Option<String>> {
    let mut candidates = vec![
        selected_model.map(str::to_owned),
        response.and_then(ResponseView::model).map(str::to_owned),
    ];
    if let Some(model) = request_model {
        candidates.push(Some(model.to_owned()));
    }
    candidates
}
