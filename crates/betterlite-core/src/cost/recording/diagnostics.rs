use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_metadata::setup::merge_litellm_metadata;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::zero_cost::{
    TokenUsage, ZeroCostDiagnostic, diagnose_zero_cost, zero_cost_warning,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::CostRecorder;
use super::identity::{self, BillingIdentity, PreparationError};
use super::view::{self, ResponseView};
use crate::cost::context::error::BillingContextError;
use crate::cost::context::pricing::pricing_entry;
use crate::cost::context::{IdentityRequest, ProviderInference};
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
        let Some((model, pricing_model, entry)) = self.priced_entry(log, input)? else {
            return Ok(None);
        };
        let tokens = token_usage(&usage);
        let Some(diagnostic) = diagnose_zero_cost(
            &tokens,
            &pricing_model,
            &pricing_json(&entry),
            input.calculation_failed,
        ) else {
            return Ok(None);
        };
        let params = identity::metadata::params_view(&log.litellm_params);
        let metadata = merge_litellm_metadata(&params);
        let provider = identity::text_field(&log.model_call_details, "custom_llm_provider")?;
        let warning = zero_cost_warning(
            &diagnostic,
            model_group(&metadata),
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
    ) -> Result<Option<(String, String, PriceRecord)>, DiagnosticError> {
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
        let custom_pricing = self.custom_pricing(log, input.view)?;
        let response = view::cost_response(input.view);
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
            self.lookup,
            self.inference,
        )?
        else {
            return Ok(None);
        };
        Ok(Some((model, pricing_model, entry)))
    }
}

pub(super) fn failure_information(
    message: &str,
    billing: Option<&BillingIdentity>,
    call_type: &str,
) -> Json {
    let mut fields = IndexMap::from([
        ("error_str".to_owned(), Json::Text(message.to_owned())),
        ("traceback_str".to_owned(), Json::Text(String::new())),
    ]);
    if let Some(billing) = billing {
        fields.insert("model".to_owned(), Json::Text(billing.model.clone()));
        fields.insert(
            "cache_hit".to_owned(),
            billing.cache_hit.map_or(Json::Null, Json::Bool),
        );
        fields.insert(
            "custom_llm_provider".to_owned(),
            text_or_null(billing.custom_llm_provider.as_deref()),
        );
        fields.insert(
            "base_model".to_owned(),
            text_or_null(billing.base_model.as_deref()),
        );
        fields.insert("call_type".to_owned(), Json::Text(call_type.to_owned()));
        fields.insert(
            "custom_pricing".to_owned(),
            Json::Bool(billing.custom_pricing),
        );
    }
    Json::Object(fields)
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

fn text_or_null(text: Option<&str>) -> Json {
    match text {
        Some(found) => Json::Text(found.to_owned()),
        None => Json::Null,
    }
}

fn model_group(metadata: &IndexMap<String, Json>) -> Option<&str> {
    metadata.get("model_group").and_then(identity::text_value)
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

// diagnose_zero_cost reads Json, and a Scalar array or object keeps only its shape.
fn pricing_json(record: &PriceRecord) -> IndexMap<String, Json> {
    record
        .iter()
        .map(|(key, value)| (key.clone(), scalar_json(value)))
        .collect()
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
