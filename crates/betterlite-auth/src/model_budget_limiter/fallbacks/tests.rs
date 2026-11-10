#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::{FallbackQuery, fallback_model_within_budget};
use crate::model_budget_limiter::tests::support::{FakeCache, amount, budgets, limiter};

const PRIMARY_SPEND: &str = "virtual_key_spend:key-1:gpt-4o:1d";
const NANO_SPEND: &str = "virtual_key_spend:key-1:gpt-4o-nano:1d";

fn chain(models: &[&str]) -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([(
        "gpt-4o".to_owned(),
        models.iter().map(|model| (*model).to_owned()).collect(),
    )])
}

fn budget_map() -> BTreeMap<String, JsonData> {
    budgets(
        r#"{
            "gpt-4o": {"max_budget": 1, "budget_duration": "1d"},
            "gpt-4o-mini": {"max_budget": 1, "budget_duration": "1d"},
            "gpt-4o-nano": {"max_budget": 1, "budget_duration": "1d"}
        }"#,
    )
}

fn query<'input>(
    budget: &'input BTreeMap<String, JsonData>,
    fallbacks: &'input BTreeMap<String, Vec<String>>,
    model: &'input str,
) -> FallbackQuery<'input> {
    FallbackQuery {
        entity_id: "key-1",
        key_alias: None,
        model_max_budget: Some(budget),
        budget_fallbacks: Some(fallbacks),
        model,
    }
}

#[tokio::test]
async fn the_first_fallback_within_budget_is_returned() {
    let cache = FakeCache::new();
    cache.preload(PRIMARY_SPEND, amount("2"));
    let subject = limiter(cache);
    let budget = budget_map();
    let fallbacks = chain(&["gpt-4o-mini", "gpt-4o-nano"]);
    let chosen = fallback_model_within_budget(&subject, query(&budget, &fallbacks, "gpt-4o")).await;
    assert_eq!(chosen.as_deref(), Some("gpt-4o-mini"));
}

#[tokio::test]
async fn a_fallback_over_budget_is_skipped_for_the_next_one() {
    let cache = FakeCache::new();
    cache.preload(PRIMARY_SPEND, amount("2"));
    cache.preload(NANO_SPEND, amount("2"));
    let subject = limiter(cache);
    let budget = budget_map();
    let fallbacks = chain(&["gpt-4o-nano", "gpt-4o-mini"]);
    let chosen = fallback_model_within_budget(&subject, query(&budget, &fallbacks, "gpt-4o")).await;
    assert_eq!(chosen.as_deref(), Some("gpt-4o-mini"));
}

#[tokio::test]
async fn no_fallback_passing_returns_none() {
    let cache = FakeCache::new();
    cache.preload(PRIMARY_SPEND, amount("2"));
    cache.preload(NANO_SPEND, amount("2"));
    let subject = limiter(cache);
    let budget = budget_map();
    let fallbacks = chain(&["gpt-4o-nano"]);
    let chosen = fallback_model_within_budget(&subject, query(&budget, &fallbacks, "gpt-4o")).await;
    assert_eq!(chosen, None);
}

#[tokio::test]
async fn a_model_without_a_chain_returns_none() {
    let subject = limiter(FakeCache::new());
    let budget = budget_map();
    let fallbacks = chain(&["gpt-4o-mini"]);
    let chosen =
        fallback_model_within_budget(&subject, query(&budget, &fallbacks, "gpt-4o-mini")).await;
    assert_eq!(chosen, None);
}

#[tokio::test]
async fn absent_fallbacks_return_none() {
    let subject = limiter(FakeCache::new());
    let budget = budget_map();
    let request = FallbackQuery {
        entity_id: "key-1",
        key_alias: None,
        model_max_budget: Some(&budget),
        budget_fallbacks: None,
        model: "gpt-4o",
    };
    let chosen = fallback_model_within_budget(&subject, request).await;
    assert_eq!(chosen, None);
}
