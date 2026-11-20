use betterlite_logging::fields::usage::Usage;
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

impl<Inference> CostRecorder<'_, Inference>
where
    Inference: ProviderInference,
{
    // Ports _record_zero_cost_diagnostic: one finding and one warning, neither of which fails the call.
    pub(super) fn record_diagnostic(&self, log: &mut LoggingState, input: DiagnosticInput<'_>) {
        if input.cost.is_none() && !input.calculation_failed {
            return;
        }
        if identity::stored_cache_hit(log) == Some(true) {
            log.model_call_details
                .insert(DIAGNOSTIC_KEY.to_owned(), Json::Null);
            return;
        }
        let found = match self.zero_cost_finding(log, &input) {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(%error, "failed to compute the zero cost diagnostic");
                return;
            }
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
    fn zero_cost_finding(
        &self,
        log: &LoggingState,
        input: &DiagnosticInput<'_>,
    ) -> Result<Option<Finding>, DiagnosticError> {
        if input.cost.is_some_and(|cost| !cost.is_zero())
            || identity::unbilled(log, input.background)
        {
            return Ok(None);
        }
        let Some(usage) = get_usage_object(&view::billing_response(input.view))? else {
            return Ok(None);
        };
        let Some((model, pricing_model, priced)) = self.priced_entry(log, input)? else {
            return Ok(None);
        };
        let tokens = token_usage(&usage);
        let Some(diagnostic) = diagnose_zero_cost(
            &tokens,
            &pricing_model,
            &pricing_json(&priced),
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
    fn priced_entry(
        &self,
        log: &LoggingState,
        input: &DiagnosticInput<'_>,
    ) -> Result<Option<(String, String, PricedModel)>, DiagnosticError> {
        let model = input
            .litellm_model_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&log.model)
            .to_owned();
        let provider = identity::text_field(&log.model_call_details, "custom_llm_provider")?;
        let base = identity::metadata::base_model(log)?;
        let router = match input.router_model_id.as_deref() {
            Some(router) if !router.is_empty() => Some(router.to_owned()),
            Some(_) | None => identity::metadata::router_model_id(log)?,
        };
        let region = identity::mantle_region(log, self.mantle)?;
        let response = view::cost_response(input.view);
        let request = IdentityRequest {
            model: Some(model.as_str()),
            response: response.as_ref(),
            base_model: base.as_deref(),
            custom_pricing: Some(self.custom_pricing(log, input.view)?),
            custom_llm_provider: provider.as_deref(),
            router_model_id: router.as_deref(),
            region_name: region.as_deref(),
        };
        let Some((pricing_model, entry)) = pricing_entry(
            &request,
            Some(&log.litellm_params),
            self.lookup,
            self.inference,
        )?
        else {
            return Ok(None);
        };
        let priced = self.priced_model(&request, log, &pricing_model, entry)?;
        Ok(Some((model, pricing_model, priced)))
    }

    // A deployment model_info keeps its own rates; only a cost-map entry takes the map's tables.
    fn priced_model(
        &self,
        request: &IdentityRequest<'_>,
        log: &LoggingState,
        key: &str,
        record: PriceRecord,
    ) -> Result<PricedModel, BillingContextError> {
        let raw_router_entry = request
            .router_model_id
            .is_some_and(|id| self.lookup.raw_cost_map_entry(id).is_some());
        let from_deployment = deployment_info(
            &PricingQuery {
                custom_pricing: request.custom_pricing,
                router_model_id: request.router_model_id,
                logging_params: Some(&log.litellm_params),
            },
            self.lookup,
        )?
        .is_some();
        let mut priced = PricedModel {
            record,
            ..PricedModel::default()
        };
        if (!from_deployment || raw_router_entry)
            && let Some(tables) = self.effective_tables(key, request.custom_llm_provider)
        {
            priced.tiered_pricing = tables.tiered_pricing;
            priced.off_peak_pricing = tables.off_peak_pricing;
        }
        Ok(priced)
    }

    // The record keeps only a nested table's shape; resolving its own key keeps the decoded rates.
    fn effective_tables(&self, key: &str, provider: Option<&str>) -> Option<PricedModel> {
        let priced = self.lookup.effective_priced_model(key, provider)?;
        matches!(
            priced.record.get("key"),
            Some(Scalar::Text(found)) if found.as_str() == key
        )
        .then_some(priced)
    }
}

pub(super) fn finding_json(finding: &Finding) -> Json {
    let keys = finding
        .diagnostic
        .missing_pricing_keys
        .iter()
        .map(|key| Json::Text((*key).to_owned()))
        .collect();
    Json::Object(IndexMap::from([
        (
            "reason".to_owned(),
            Json::Text(finding.diagnostic.reason.label().to_owned()),
        ),
        (
            "pricing_model".to_owned(),
            Json::Text(finding.diagnostic.pricing_model.clone()),
        ),
        ("missing_pricing_keys".to_owned(), Json::List(keys)),
    ]))
}

fn token_usage(usage: &Usage) -> TokenUsage {
    TokenUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        prompt_audio_tokens: audio_tokens(usage.prompt_tokens_details.as_ref()),
        completion_audio_tokens: audio_tokens(usage.completion_tokens_details.as_ref()),
    }
}

fn audio_tokens(details: Option<&IndexMap<String, Json>>) -> i64 {
    match details.and_then(|found| found.get("audio_tokens")) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
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
    let values = rates.flatten().flatten().map(scalar_json).collect();
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
