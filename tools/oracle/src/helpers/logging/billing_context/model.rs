use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::cost::context::CostResponse;
use betterlite_logging::cost_breakdown::CostBreakdownInput;
use betterlite_logging::fields::timestamps::Moment;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;

use super::{
    BreakdownInput, CatalogEntry, Failure, RatesInput, ResponseInput, ResponseKind, failure,
};

pub(super) struct CaseLookup {
    raw: BTreeMap<String, PriceRecord>,
}

impl From<&[CatalogEntry]> for CaseLookup {
    fn from(catalog: &[CatalogEntry]) -> Self {
        Self {
            raw: catalog
                .iter()
                .map(|entry| (entry.key.clone(), entry.entry.clone()))
                .collect(),
        }
    }
}

impl PriceLookup for CaseLookup {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.raw.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.raw.get(key).cloned()
    }

    // The identity and deployment views never read tiers, so the case catalog keeps none.
    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

pub(super) fn cost_response(input: &ResponseInput) -> CostResponse<'_> {
    match input.kind {
        ResponseKind::Object => CostResponse::Object {
            model: input.model.as_deref(),
            hidden_params: input.hidden_params.as_ref(),
            response_ms: input.response_ms.as_ref(),
        },
        ResponseKind::Dictionary => CostResponse::Dictionary {
            model: input.model.as_deref(),
        },
    }
}

pub(super) fn logging_state() -> Result<LoggingState, Failure> {
    let init = LoggingInit {
        model: "billing-context".to_owned(),
        call_type: "completion".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").map_err(failure)?,
        litellm_call_id: "call-1".to_owned(),
        function_id: "fn-1".to_owned(),
        messages: None,
        stream: None,
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let mut context = CorrelationContext::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "trace-1".to_owned(),
        Duration::default,
    )
    .map_err(failure)
}

pub(super) fn moment(text: &str) -> Result<Moment, Failure> {
    DateTime::parse_from_rfc3339(text)
        .map(Moment::DateTime)
        .map_err(failure)
}

pub(super) fn breakdown_input(input: &BreakdownInput) -> CostBreakdownInput {
    CostBreakdownInput {
        input_cost: input.input_cost,
        output_cost: input.output_cost,
        total_cost: input.total_cost,
        tool_usage_cost: input.tool_usage_cost,
        additional_costs: input.additional_costs.clone(),
        original_cost: input.original_cost,
        discount_percent: input.discount_percent,
        discount_amount: input.discount_amount,
        margin_percent: input.margin_percent,
        margin_fixed_amount: input.margin_fixed_amount,
        margin_total_amount: input.margin_total_amount,
        cache_read_cost: input.cache_read_cost,
        cache_creation_cost: input.cache_creation_cost,
        reasoning_cost: input.reasoning_cost,
        service_tier: input.service_tier.clone(),
        data_residency: input.data_residency.clone(),
        vertex_location: input.vertex_location.clone(),
        billed_token_rates: input.billed_token_rates.map(rates),
    }
}

const fn rates(input: RatesInput) -> BilledTokenRates {
    BilledTokenRates {
        input_cost_per_token: input.input_cost_per_token,
        output_cost_per_token: input.output_cost_per_token,
        cache_read_input_token_cost: input.cache_read_input_token_cost,
        audio_read: input.audio_read,
        cache_write: input.cache_write,
        cache_write_1h: input.cache_write_1h,
        reasoning: input.reasoning,
    }
}
