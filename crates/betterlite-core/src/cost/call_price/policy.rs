pub mod error;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use rust_decimal::Decimal;

use crate::routes::CallType;
use error::PolicyError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexRoute {
    Character,
    Token,
}

pub struct TokenPriceRequest<'request> {
    pub provider: &'request str,
    pub model: &'request str,
    pub usage: &'request CostUsage,
    pub prompt_extras: &'request PromptExtras,
    pub service_tier: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
}

pub struct CharacterPriceRequest<'request> {
    pub provider: &'request str,
    pub model: &'request str,
    pub prompt_characters: Option<i64>,
    pub completion_characters: Option<i64>,
    pub usage: &'request CostUsage,
    pub prompt_extras: &'request PromptExtras,
    pub service_tier: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
}

pub struct SearchPriceRequest<'request> {
    pub provider: &'request str,
    pub model: &'request str,
    pub number_of_queries: i64,
    pub hidden_params: Option<&'request PriceRecord>,
}

pub struct RerankPriceRequest<'request> {
    pub provider: &'request str,
    pub model: &'request str,
    pub billed_units: Option<&'request RerankBilledUnits>,
    pub model_info: Option<&'request PriceRecord>,
}

pub struct VectorStorePriceRequest<'request> {
    pub provider: &'request str,
    pub api_type: Option<&'request str>,
    pub response: &'request VectorStoreSearchResponse,
}

// Each default is the typed outcome for a provider nobody installed; an installed provider overrides only the operations it prices.
pub trait ProviderPricePolicy {
    fn vertex_route(&self, _model: &str, _call_type: CallType) -> Result<VertexRoute, PolicyError> {
        Err(PolicyError::UnavailableAdapter("vertex_ai".to_owned()))
    }

    fn lyria_cost(&self, provider: &str, _model: &str) -> Result<Option<Decimal>, PolicyError> {
        Err(PolicyError::UnavailableAdapter(provider.to_owned()))
    }

    fn token_cost(
        &self,
        request: &TokenPriceRequest<'_>,
    ) -> Result<GenericCostResult, PolicyError> {
        Err(PolicyError::UnavailableAdapter(request.provider.to_owned()))
    }

    fn character_cost(
        &self,
        request: &CharacterPriceRequest<'_>,
    ) -> Result<GenericCostResult, PolicyError> {
        Err(PolicyError::UnavailableAdapter(request.provider.to_owned()))
    }

    fn search_cost(
        &self,
        request: &SearchPriceRequest<'_>,
    ) -> Result<GenericCostResult, PolicyError> {
        Err(PolicyError::UnavailableAdapter(request.provider.to_owned()))
    }

    fn rerank_cost(
        &self,
        request: &RerankPriceRequest<'_>,
    ) -> Result<GenericCostResult, PolicyError> {
        Err(PolicyError::UnavailableAdapter(request.provider.to_owned()))
    }

    // None means the provider has no vector-store configuration, which prices at zero.
    fn vector_store_cost(
        &self,
        request: &VectorStorePriceRequest<'_>,
    ) -> Result<Option<GenericCostResult>, PolicyError> {
        Err(PolicyError::UnavailableAdapter(request.provider.to_owned()))
    }
}
