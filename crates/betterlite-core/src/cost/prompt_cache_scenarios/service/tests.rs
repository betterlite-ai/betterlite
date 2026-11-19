#![cfg(test)]

use std::str::FromStr as _;

use betterlite_anthropic::cache_aware_routing::services::CacheCostService;
use betterlite_anthropic::cache_aware_routing::services::MinimumCacheTokens;
use betterlite_anthropic::cache_aware_routing::services::ScenarioRequest;
use betterlite_models::catalog::BundledCatalog;
use rstest::rstest;
use rust_decimal::Decimal;

use super::CatalogCacheCostService;
use super::PromptCacheMinimum;
use crate::cost::prompt_cache_scenarios::test_support::DEPLOYMENT_ID;
use crate::cost::prompt_cache_scenarios::test_support::Lookup;
use crate::cost::prompt_cache_scenarios::test_support::MODEL;
use crate::cost::prompt_cache_scenarios::test_support::Prices;
use crate::cost::prompt_cache_scenarios::test_support::flat_tariff;
use crate::cost::prompt_cache_scenarios::test_support::moment;
use crate::cost::prompt_cache_scenarios::test_support::request;
use crate::cost::prompt_cache_scenarios::test_support::tokens;
use crate::cost::prompt_cache_scenarios::token_buckets::token_buckets;
use crate::message_utilities::prompt_cache_checks::PromptCacheSettings;

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a decimal literal")
}

fn tariffed() -> Prices {
    Prices::new(Lookup::new(&[(DEPLOYMENT_ID, flat_tariff())]))
}

#[rstest]
#[case::five_minutes(300)]
#[case::one_hour(3600)]
fn the_catalog_service_buckets_like_token_buckets(#[case] ttl: u32) {
    let prices = tariffed();
    let service = CatalogCacheCostService {
        environment: prices.environment(),
    };
    assert_eq!(
        service.buckets(100, 20, 40, ttl),
        token_buckets(100, 20, 40, ttl)
    );
}

#[test]
fn the_catalog_service_prices_a_scenario() {
    let prices = tariffed();
    let service = CatalogCacheCostService {
        environment: prices.environment(),
    };
    let tokens = tokens(1000, 0, 0, 0);
    let scenario = service
        .scenario(
            &ScenarioRequest {
                model: MODEL,
                deployment_id: DEPLOYMENT_ID,
                tokens: &tokens,
            },
            moment(),
        )
        .expect("a priced scenario");
    assert_eq!(scenario.tokens, tokens);
    assert_eq!(scenario.input_cost, dec("1"));
}

#[test]
fn the_catalog_service_prices_the_completion_tokens_too() {
    let prices = tariffed();
    let service = CatalogCacheCostService {
        environment: prices.environment(),
    };
    let tokens = tokens(1000, 0, 0, 0);
    assert_eq!(
        service.price(&request(&tokens, 500), moment()),
        Some(dec("1.5"))
    );
}

#[test]
fn the_minimum_reads_the_catalog_entry() {
    let catalog = BundledCatalog::default().get();
    let minimum = PromptCacheMinimum {
        settings: PromptCacheSettings::default(),
        catalog: catalog.as_ref(),
    };
    assert_eq!(minimum.minimum_tokens("claude-fable-5"), 512);
}

#[test]
fn the_minimum_reads_the_configured_override() {
    let catalog = BundledCatalog::default().get();
    let minimum = PromptCacheMinimum {
        settings: PromptCacheSettings {
            minimum_token_override: Some(7),
        },
        catalog: catalog.as_ref(),
    };
    assert_eq!(minimum.minimum_tokens("claude-fable-5"), 7);
}
