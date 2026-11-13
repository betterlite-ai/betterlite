use std::fmt::Debug;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::resolution::ResolvedModel;
use betterlite_models::token_cost::GenericCostResult;
use indexmap::IndexMap;

use crate::cost::call_price::policy::error::PolicyError;
use crate::routes::CallType;

pub struct SearchQuery<'query> {
    pub model: &'query str,
    pub provider: Option<&'query str>,
    pub number_of_queries: i64,
    pub optional_params: Option<&'query IndexMap<String, Json>>,
}

// get_llm_provider as the cost stage reads it: the stripped model and the provider, injected so this stage reads no global catalog.
pub trait ModelResolver {
    type Failure: Debug;

    fn resolve_model(&self, model: &str) -> Result<ResolvedModel, Self::Failure>;
}

// The provider-owned operations of the early-return stage; an unbuilt adapter answers with its typed error.
pub trait SpecialPricePolicy {
    fn search_cost(&self, query: &SearchQuery<'_>) -> Result<GenericCostResult, PolicyError> {
        Err(PolicyError::UnavailableAdapter(
            query.provider.unwrap_or("search").to_owned(),
        ))
    }

    fn has_together_registry_pricing(
        &self,
        _model: &str,
        _lookup: &dyn PriceLookup,
    ) -> Result<bool, PolicyError> {
        Err(PolicyError::UnavailableAdapter("together_ai".to_owned()))
    }

    fn together_category(&self, _model: &str, _call_type: CallType) -> Result<String, PolicyError> {
        Err(PolicyError::UnavailableAdapter("together_ai".to_owned()))
    }
}
