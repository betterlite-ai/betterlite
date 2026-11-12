#![cfg(test)]

use betterlite_core::cost::call_price::policy::error::PolicyError;
use betterlite_core::cost::call_price::policy::{
    CharacterPriceRequest, ProviderPricePolicy as _, RerankPriceRequest, SearchPriceRequest,
    TokenPriceRequest, VectorStorePriceRequest,
};
use betterlite_core::routes::CallType;
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::usage_sum::Usage;

use super::ProviderPricing;

fn unavailable(error: PolicyError) -> String {
    match error {
        PolicyError::UnavailableAdapter { provider } => provider,
        PolicyError::Failed { .. } => "failed".to_owned(),
    }
}

#[test]
fn an_uninstalled_token_or_character_formula_is_a_typed_unavailable_adapter() {
    let usage = CostUsage::from(&Usage::default());
    let extras = PromptExtras::default();
    let token = TokenPriceRequest {
        provider: "deepseek",
        model: "chat",
        usage: &usage,
        prompt_extras: &extras,
        service_tier: None,
        vertex_location: None,
    };
    let character = CharacterPriceRequest {
        provider: "vertex_ai",
        model: "gemini",
        prompt_characters: Some(1),
        completion_characters: Some(2),
        usage: &usage,
        prompt_extras: &extras,
        service_tier: None,
        vertex_location: None,
    };
    let pricing = ProviderPricing;
    assert_eq!(
        unavailable(pricing.token_cost(&token).unwrap_err()),
        "deepseek"
    );
    assert_eq!(
        unavailable(pricing.character_cost(&character).unwrap_err()),
        "vertex_ai"
    );
}

#[test]
fn an_uninstalled_search_rerank_and_vector_formula_is_a_typed_unavailable_adapter() {
    let search = SearchPriceRequest {
        provider: "tavily",
        model: "search",
        number_of_queries: 1,
        hidden_params: None,
    };
    let rerank = RerankPriceRequest {
        provider: "cohere",
        model: "rerank-v3",
        billed_units: None,
        model_info: None,
    };
    let response =
        serde_json::from_str(r#"{"object": "o", "search_query": "q", "data": []}"#).unwrap();
    let vector = VectorStorePriceRequest {
        provider: "openai",
        api_type: None,
        response: &response,
    };
    let pricing = ProviderPricing;
    assert_eq!(
        unavailable(pricing.search_cost(&search).unwrap_err()),
        "tavily"
    );
    assert_eq!(
        unavailable(pricing.rerank_cost(&rerank).unwrap_err()),
        "cohere"
    );
    assert_eq!(
        unavailable(pricing.vector_store_cost(&vector).unwrap_err()),
        "openai"
    );
}

#[test]
fn an_uninstalled_vertex_route_and_lyria_formula_name_their_provider() {
    let pricing = ProviderPricing;
    assert_eq!(
        unavailable(
            pricing
                .vertex_route("gemini", CallType::Completion)
                .unwrap_err()
        ),
        "vertex_ai"
    );
    assert_eq!(
        unavailable(pricing.lyria_cost("vertex_ai_beta", "lyria").unwrap_err()),
        "vertex_ai_beta"
    );
}
