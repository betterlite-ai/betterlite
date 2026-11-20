use std::mem::take;
use std::str::FromStr as _;
use std::sync::Mutex;

use betterlite_core::cost::prepare::ResponseView;
use betterlite_core::cost::recording::{CostCalculation, CostRequest, PortFailure};
use betterlite_core::cost::usage::{BillingResponse, BillingUsage, ResponseKind, get_usage_object};
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Serialize;

use super::input::CalculatorSpec;
use super::providers::FailureText;

#[derive(Debug, Default, PartialEq, Serialize)]
pub(super) struct Observed {
    model: String,
    cache_hit: Option<bool>,
    provider: Option<String>,
    base_model: Option<String>,
    call_type: String,
    custom_pricing: bool,
    prompt: String,
    router_model_id: Option<String>,
    data_residency: Option<String>,
    vertex_location: Option<String>,
    region_name: Option<String>,
    usage: Option<(i64, i64)>,
}

pub(super) struct Calculator {
    result: Result<Decimal, FailureText>,
    observed: Mutex<Vec<Observed>>,
}

impl Calculator {
    pub(super) fn new(spec: &CalculatorSpec) -> Self {
        let result = match &spec.error {
            Some(message) => Err(FailureText(message.clone())),
            None => cost_value(spec.cost.as_ref()),
        };
        Self {
            result,
            observed: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn observe(&self) -> Vec<Observed> {
        match self.observed.lock() {
            Ok(mut found) => take(&mut *found),
            Err(poisoned) => take(&mut poisoned.into_inner()),
        }
    }
}

fn cost_value(value: Option<&Json>) -> Result<Decimal, FailureText> {
    match value {
        None | Some(Json::Null) => Ok(Decimal::ZERO),
        Some(Json::Number(number)) => {
            Decimal::from_str(&number.to_string()).map_err(|error| FailureText(error.to_string()))
        }
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => Err(FailureText(
            "the calculator cost must be a number".to_owned(),
        )),
    }
}

#[async_trait::async_trait]
impl CostCalculation for Calculator {
    async fn calculate(&self, request: &CostRequest<'_>) -> Result<Decimal, PortFailure> {
        let usage = usage_counts(request);
        {
            let mut observed = match self.observed.lock() {
                Ok(found) => found,
                Err(poisoned) => poisoned.into_inner(),
            };
            observed.push(Observed {
                model: request.model.to_owned(),
                cache_hit: request.cache_hit,
                provider: request.custom_llm_provider.map(str::to_owned),
                base_model: request.base_model.map(str::to_owned),
                call_type: request.call_type.to_owned(),
                custom_pricing: request.custom_pricing,
                prompt: request.prompt.to_owned(),
                router_model_id: request.router_model_id.map(str::to_owned),
                data_residency: request.data_residency.map(str::to_owned),
                vertex_location: request.vertex_location.map(str::to_owned),
                region_name: request.region_name.map(str::to_owned),
                usage,
            });
        }
        self.result
            .as_ref()
            .map(|cost| *cost)
            .map_err(|error| PortFailure::from(error.clone()))
    }
}

fn usage_counts(request: &CostRequest<'_>) -> Option<(i64, i64)> {
    let billing = match request.response {
        ResponseView::Typed(typed) => BillingResponse {
            kind: ResponseKind::Chat,
            usage: reborrow(&typed.usage),
            service_tier: typed.service_tier,
            hidden_params: typed.hidden_params,
        },
        ResponseView::Dictionary(dictionary) => BillingResponse {
            kind: ResponseKind::Chat,
            usage: reborrow(&dictionary.usage),
            service_tier: dictionary.service_tier,
            hidden_params: None,
        },
        ResponseView::Realtime { usage, .. } => BillingResponse {
            kind: ResponseKind::Chat,
            usage: BillingUsage::Chat(usage),
            service_tier: None,
            hidden_params: None,
        },
        ResponseView::Other => return None,
    };
    match get_usage_object(&billing) {
        Ok(Some(found)) => Some((found.prompt_tokens, found.completion_tokens)),
        Ok(None) | Err(_) => None,
    }
}

const fn reborrow<'usage>(usage: &BillingUsage<'usage>) -> BillingUsage<'usage> {
    match usage {
        BillingUsage::Absent => BillingUsage::Absent,
        BillingUsage::Null => BillingUsage::Null,
        BillingUsage::Chat(inner) => BillingUsage::Chat(*inner),
        BillingUsage::Response(inner) => BillingUsage::Response(*inner),
        BillingUsage::Transcription(inner) => BillingUsage::Transcription(*inner),
        BillingUsage::DecisionsIr(inner) => BillingUsage::DecisionsIr(*inner),
        BillingUsage::DecisionsOpenAi(inner) => BillingUsage::DecisionsOpenAi(*inner),
        BillingUsage::Interactions(inner) => BillingUsage::Interactions(*inner),
        BillingUsage::Mapping(inner) => BillingUsage::Mapping(*inner),
        BillingUsage::TypedRecord(inner) => BillingUsage::TypedRecord(*inner),
    }
}
