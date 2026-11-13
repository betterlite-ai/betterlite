#![cfg(test)]

use betterlite_core::cost::call_price::policy::error::PolicyError;
use betterlite_core::cost::special::policy::{SearchQuery, SpecialPricePolicy as _};
use betterlite_core::routes::CallType;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use rstest::rstest;

use super::ProviderPricing;

struct EmptyCatalog;

impl PriceLookup for EmptyCatalog {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

fn unavailable(error: PolicyError) -> String {
    match error {
        PolicyError::UnavailableAdapter(adapter) => adapter,
    }
}

#[rstest]
#[case::named_provider(Some("tavily"), "tavily")]
#[case::no_provider(None, "search")]
fn an_uninstalled_search_formula_is_a_typed_unavailable_adapter(
    #[case] provider: Option<&str>,
    #[case] adapter: &str,
) {
    let query = SearchQuery {
        model: "tavily/search",
        provider,
        number_of_queries: 1,
        optional_params: None,
    };
    let error = ProviderPricing.search_cost(&query).unwrap_err();
    assert_eq!(unavailable(error), adapter);
}

#[test]
fn an_uninstalled_together_operation_is_a_typed_unavailable_adapter() {
    let registry =
        ProviderPricing.has_together_registry_pricing("together_ai/llama", &EmptyCatalog);
    assert_eq!(unavailable(registry.unwrap_err()), "together_ai");
    let category = ProviderPricing.together_category("together_ai/llama", CallType::Completion);
    assert_eq!(unavailable(category.unwrap_err()), "together_ai");
}
