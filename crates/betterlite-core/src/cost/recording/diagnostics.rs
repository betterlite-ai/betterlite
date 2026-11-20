use betterlite_logging::logging_metadata::setup::merge_litellm_metadata;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::off_peak::schema::RateField;
use betterlite_models::price_lookup::{PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::zero_cost::{
    TokenUsage, ZeroCostDiagnostic, diagnose_zero_cost, zero_cost_warning,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::CostRecorder;
use super::identity::{self, PreparationError};
use super::view;
use crate::cost::context::error::BillingContextError;
use crate::cost::context::pricing::{PricingQuery, deployment_info, pricing_entry};
use crate::cost::context::{IdentityRequest, ProviderInference};
use crate::cost::prepare::ResponseView;
use crate::cost::usage::{BillingUsageError, get_usage_object};

const DIAGNOSTIC_KEY: &str = "zero_cost_diagnostic";
pub(super) const FAILURE_KEY: &str = "response_cost_failure_debug_information";
type ModelInfo = IndexMap<String, Json>;
type PricedEntry = (String, String, PricedModel, Option<ModelInfo>);

#[derive(Debug, thiserror::Error)]
pub(super) enum DiagnosticError {
    #[error(transparent)]
    Pricing(#[from] BillingContextError),
    #[error(transparent)]
    Preparation(#[from] PreparationError),
    #[error(transparent)]
    Usage(#[from] BillingUsageError),
}

pub(super) struct Finding {
    pub(super) diagnostic: ZeroCostDiagnostic,
    pub(super) warning: String,
}

// Everything _record_zero_cost_diagnostic and _zero_cost_finding read from the request.
pub(super) struct DiagnosticInput<'view> {
    pub(super) view: &'view ResponseView<'view>,
    pub(super) background: bool,
    pub(super) cost: Option<Decimal>,
    pub(super) calculation_failed: bool,
    pub(super) litellm_model_name: Option<String>,
    pub(super) router_model_id: Option<String>,
}

// Ports _record_zero_cost_diagnostic: one finding and one warning, neither of which fails the call.
pub(super) fn record_diagnostic<Inference: ProviderInference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    input: &DiagnosticInput<'_>,
) {
    if input.cost.is_none() && !input.calculation_failed {
        return;
    }
    if identity::stored_cache_hit(log) == Some(true) {
        log.model_call_details
            .insert(DIAGNOSTIC_KEY.to_owned(), Json::Null);
        return;
    }
    let Ok(found) = zero_cost_finding(recorder, log, input).inspect_err(|error| {
        tracing::debug!(%error, "failed to compute the zero cost diagnostic");
    }) else {
        return;
    };
    if let Some(finding) = &found
        && !log.cost.zero_cost_warned
    {
        log.cost.zero_cost_warned = true;
        tracing::warn!("{}", finding.warning);
    }
    let value = found.map_or(Json::Null, |finding| finding_json(&finding));
    log.model_call_details
        .insert(DIAGNOSTIC_KEY.to_owned(), value);
}

// Ports _zero_cost_finding: zero or unbilled costs only, priced through B1682 and B1683.
fn zero_cost_finding<Inference: ProviderInference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &LoggingState,
    input: &DiagnosticInput<'_>,
) -> Result<Option<Finding>, DiagnosticError> {
    if input.cost.is_some_and(|cost| !cost.is_zero()) || identity::unbilled(log, input.background) {
        return Ok(None);
    }
    let Some(usage) = get_usage_object(&view::billing_response(input.view))? else {
        return Ok(None);
    };
    let Some((model, pricing_model, priced, raw_info)) = priced_entry(recorder, log, input)? else {
        return Ok(None);
    };
    let tokens = TokenUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        prompt_audio_tokens: audio_tokens(usage.prompt_tokens_details.as_ref()),
        completion_audio_tokens: audio_tokens(usage.completion_tokens_details.as_ref()),
    };
    let Some(diagnostic) = diagnose_zero_cost(
        &tokens,
        &pricing_model,
        &raw_info.unwrap_or_else(|| pricing_json(&priced)),
        input.calculation_failed,
    ) else {
        return Ok(None);
    };
    let params = identity::metadata::params_view(&log.litellm_params);
    let metadata = merge_litellm_metadata(&params);
    let provider = identity::text_field(&log.model_call_details, "custom_llm_provider")?;
    let warning = zero_cost_warning(
        &diagnostic,
        metadata.get("model_group").and_then(identity::text_value),
        &model,
        provider.as_deref(),
        &tokens,
    );
    Ok(Some(Finding {
        diagnostic,
        warning,
    }))
}

// The pricing identity _zero_cost_finding hands to B1682, resolved before the price entry.
fn priced_entry<Inference: ProviderInference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &LoggingState,
    input: &DiagnosticInput<'_>,
) -> Result<Option<PricedEntry>, DiagnosticError> {
    let named = input
        .litellm_model_name
        .as_deref()
        .filter(|name| !name.is_empty());
    let model = named.unwrap_or(&log.model).to_owned();
    let provider = identity::text_field(&log.model_call_details, "custom_llm_provider")?;
    let base = identity::metadata::base_model(log)?;
    let router = match input.router_model_id.as_deref().filter(|id| !id.is_empty()) {
        Some(router) => Some(router.to_owned()),
        None => identity::metadata::router_model_id(log)?,
    };
    let region = identity::mantle_region(log, recorder.mantle)?;
    let response = view::cost_response(input.view);
    let custom_pricing = identity::metadata::custom_pricing(recorder, log, input.view)?;
    let request = IdentityRequest {
        model: Some(model.as_str()),
        response: response.as_ref(),
        base_model: base.as_deref(),
        custom_pricing: Some(custom_pricing),
        custom_llm_provider: provider.as_deref(),
        router_model_id: router.as_deref(),
        region_name: region.as_deref(),
    };
    let Some((pricing_model, entry)) = pricing_entry(
        &request,
        Some(&log.litellm_params),
        recorder.lookup,
        recorder.inference,
    )?
    else {
        return Ok(None);
    };
    let (priced, raw_info) = priced_model(recorder, &request, log, &pricing_model, entry)?;
    Ok(Some((model, pricing_model, priced, raw_info)))
}

// A deployment model_info keeps its own rates; only a cost-map entry takes the map's tables.
fn priced_model<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    request: &IdentityRequest<'_>,
    log: &LoggingState,
    key: &str,
    record: PriceRecord,
) -> Result<(PricedModel, Option<ModelInfo>), DiagnosticError> {
    let raw_router_entry = request
        .router_model_id
        .is_some_and(|id| recorder.lookup.raw_cost_map_entry(id).is_some());
    let from_deployment = deployment_info(
        &PricingQuery {
            custom_pricing: request.custom_pricing,
            router_model_id: request.router_model_id,
            logging_params: Some(&log.litellm_params),
        },
        recorder.lookup,
    )?
    .is_some();
    let mut priced = PricedModel {
        record,
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    };
    let mut raw_info: Option<ModelInfo> = None;
    let deployment_rates = from_deployment && !raw_router_entry;
    if deployment_rates {
        raw_info = ["metadata", "litellm_metadata"].iter().find_map(|field| {
            let metadata = identity::object_value(log.litellm_params.get(*field)?)?;
            identity::object_value(metadata.get("model_info")?).cloned()
        });
    }
    if !deployment_rates
        && let Some(tables) = effective_tables(recorder, key, request.custom_llm_provider)
    {
        priced.tiered_pricing = tables.tiered_pricing;
        priced.off_peak_pricing = tables.off_peak_pricing;
    }
    Ok((priced, raw_info))
}

// The record keeps only a nested table's shape; resolving its own key keeps the decoded rates.
fn effective_tables<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    key: &str,
    provider: Option<&str>,
) -> Option<PricedModel> {
    let priced = recorder.lookup.effective_priced_model(key, provider)?;
    let Some(Scalar::Text(found)) = priced.record.get("key") else {
        return None;
    };
    (found.as_str() == key).then_some(priced)
}

pub(super) fn finding_json(finding: &Finding) -> Json {
    let reason = Json::Text(finding.diagnostic.reason.label().to_owned());
    let pricing_model = Json::Text(finding.diagnostic.pricing_model.clone());
    let keys = finding
        .diagnostic
        .missing_pricing_keys
        .iter()
        .map(|key| Json::Text((*key).to_owned()))
        .collect();
    Json::Object(IndexMap::from([
        ("reason".to_owned(), reason),
        ("pricing_model".to_owned(), pricing_model),
        ("missing_pricing_keys".to_owned(), Json::List(keys)),
    ]))
}

fn audio_tokens(details: Option<&IndexMap<String, Json>>) -> i64 {
    let Some(Json::Number(number)) = details.and_then(|found| found.get("audio_tokens")) else {
        return 0;
    };
    number.as_i64().unwrap_or(0)
}

// The record keeps only each table's shape, so diagnose_zero_cost gets the decoded rates beside it.
fn pricing_json(priced: &PricedModel) -> IndexMap<String, Json> {
    let mut entry: IndexMap<String, Json> = priced
        .record
        .iter()
        .map(|(key, value)| (key.clone(), scalar_json(value)))
        .collect();
    let rates = priced.tiered_pricing.iter().flat_map(|tier| {
        [
            tier.input_token.as_ref(),
            tier.output_token.as_ref(),
            tier.output_reasoning_token.as_ref(),
            tier.cache_read_input_token_cost.as_ref(),
            tier.cache_creation_token_cost.as_ref(),
            tier.cache_creation_above_1hr_cost.as_ref(),
            tier.input_cost_per_query.as_ref(),
        ]
    });
    let values = rates.flatten().map(scalar_json).collect();
    entry.insert("tiered_pricing".to_owned(), Json::List(values));
    if let Some(block) = &priced.off_peak_pricing {
        let block_rates = [
            block.input_cost_per_token.as_ref(),
            block.output_cost_per_token.as_ref(),
            block.reasoning_token_cost.as_ref(),
            block.cache_read_input_token_cost.as_ref(),
            block.cache_creation_cost.as_ref(),
        ];
        let peak_values = block_rates.into_iter().flatten().map(rate_json).collect();
        entry.insert("off_peak_pricing".to_owned(), Json::List(peak_values));
    }
    entry
}

fn rate_json(rate: &RateField) -> Json {
    match rate {
        RateField::Number(number) => Json::Number(number.clone()),
        RateField::Bool(_) | RateField::Text(_) | RateField::Other(_) => Json::Null,
    }
}

fn scalar_json(value: &Scalar) -> Json {
    match value {
        Scalar::Null => Json::Null,
        Scalar::Bool(flag) => Json::Bool(*flag),
        Scalar::Number(number) => Json::Number(number.clone()),
        Scalar::Text(text) => Json::Text(text.clone()),
        Scalar::Array(items) => Json::List(vec![Json::Null; items.len()]),
        Scalar::Object(_) => Json::Object(IndexMap::new()),
    }
}
