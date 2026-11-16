#![cfg(test)]

use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_error::rate_limit::RateLimitDimension;
use betterlite_models::decisions::Json;
use betterlite_redact::structured::Payload;

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::session_budget::SessionBudgetHandler;

use crate::session_budget::error::SessionBudgetError;
use crate::session_budget::tests::support::BUDGET_FALSE;
use crate::session_budget::tests::support::BUDGET_INFINITY;
use crate::session_budget::tests::support::BUDGET_INVALID;
use crate::session_budget::tests::support::BUDGET_LIST;
use crate::session_budget::tests::support::BUDGET_NULL;
use crate::session_budget::tests::support::BUDGET_ONE;
use crate::session_budget::tests::support::BUDGET_ROUNDING;
use crate::session_budget::tests::support::BUDGET_TEXT;
use crate::session_budget::tests::support::BUDGET_TRUE;
use crate::session_budget::tests::support::BUDGET_ZERO;
use crate::session_budget::tests::support::FakeCache;
use crate::session_budget::tests::support::NO_BUDGET;
use crate::session_budget::tests::support::agent;
use crate::session_budget::tests::support::agent_registry_with;
use crate::session_budget::tests::support::amount;
use crate::session_budget::tests::support::handler;
use crate::session_budget::tests::support::json;
use crate::session_budget::tests::support::rejection;

const SPEND_KEY: &str = "{session_budget:s1}:spend";
const REQUEST: &str = r#"{"model":"gpt-4o","metadata":{"session_id":"s1"}}"#;

async fn check(
    handler: &SessionBudgetHandler<FakeCache>,
    registry: &AgentRegistry,
    agent_id: Option<&str>,
    data: &Json,
) -> Result<(), SessionBudgetError> {
    handler
        .async_pre_call_hook(registry, None, agent_id, data)
        .await
}

fn over_budget_cache() -> FakeCache {
    let cache = FakeCache::new();
    cache.preload_memory(SPEND_KEY, amount("1.0"));
    cache
}

#[tokio::test]
async fn admission_rejects_spend_equal_to_the_budget() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    assert_eq!(
        rejection(error).rate_limit_type,
        Some(RateLimitDimension::Budget)
    );
}

#[tokio::test]
async fn admission_rejects_spend_above_the_budget() {
    let cache = FakeCache::new();
    cache.preload_memory(SPEND_KEY, amount("2.0"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_ONE]);
    assert!(
        check(&handler, &registry, Some("agent-1"), &json(REQUEST))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn admission_allows_spend_below_the_budget() {
    let cache = FakeCache::new();
    cache.preload_memory(SPEND_KEY, amount("0.999"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_ONE]);
    assert!(
        check(&handler, &registry, Some("agent-1"), &json(REQUEST))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn admission_allows_a_session_with_no_recorded_spend() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    assert!(
        check(&handler, &registry, Some("agent-1"), &json(REQUEST))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn admission_rejects_at_an_explicit_zero_budget() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ZERO]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    assert_eq!(
        rejection(error).rate_limit_type,
        Some(RateLimitDimension::Budget)
    );
}

#[tokio::test]
async fn admission_allows_an_agent_without_the_budget_key() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[NO_BUDGET]);
    assert!(
        check(&handler, &registry, Some("agent-2"), &json(REQUEST))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn admission_allows_an_unregistered_agent() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    assert!(
        check(&handler, &registry, Some("agent-9"), &json(REQUEST))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn admission_allows_a_request_without_an_agent_id() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    assert!(
        check(&handler, &registry, None, &json(REQUEST))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn admission_treats_a_numeric_text_budget_as_a_number() {
    let cache = FakeCache::new();
    cache.preload_memory(SPEND_KEY, amount("0.5"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_TEXT]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    assert_eq!(
        rejection(error).rate_limit_type,
        Some(RateLimitDimension::Budget)
    );
}

#[tokio::test]
async fn admission_treats_a_true_budget_as_one_dollar() {
    let cache = FakeCache::new();
    cache.preload_memory(SPEND_KEY, amount("1.0"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_TRUE]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    assert_eq!(
        rejection(error).rate_limit_type,
        Some(RateLimitDimension::Budget)
    );
}

#[tokio::test]
async fn admission_allows_a_null_budget() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_NULL]);
    assert!(
        check(&handler, &registry, Some("agent-1"), &json(REQUEST))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn admission_rejects_a_budget_that_is_not_a_number() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_INVALID]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    let SessionBudgetError::InvalidBudget { agent_id } = error else {
        panic!("expected an invalid budget error");
    };
    assert_eq!(agent_id, "agent-1");
}

#[tokio::test]
async fn admission_treats_a_false_budget_as_zero() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_FALSE]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    assert_eq!(
        rejection(error).rate_limit_type,
        Some(RateLimitDimension::Budget)
    );
}

#[tokio::test]
async fn admission_rejects_a_list_budget() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_LIST]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    let SessionBudgetError::InvalidBudget { agent_id } = error else {
        panic!("expected an invalid budget error");
    };
    assert_eq!(agent_id, "agent-1");
}

#[tokio::test]
async fn admission_rejects_a_non_finite_budget() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_INFINITY]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    let SessionBudgetError::InvalidBudget { agent_id } = error else {
        panic!("expected an invalid budget error");
    };
    assert_eq!(agent_id, "agent-1");
}

#[tokio::test]
async fn agent_registry_lookup_is_live_at_pre_call() {
    let handler = handler(FakeCache::new());
    let registry = AgentRegistry::default();
    let data = json(REQUEST);
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_ok()
    );
    registry.register_agent(agent(BUDGET_ZERO));
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn metadata_session_id_selects_the_cached_spend() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"gpt-4o","metadata":{"session_id":"s1"}}"#);
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn metadata_falls_back_to_litellm_metadata() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"gpt-4o","litellm_metadata":{"session_id":"s1"}}"#);
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn metadata_metadata_bucket_wins_over_litellm_metadata() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(
        r#"{"model":"gpt-4o","metadata":{"session_id":"fresh"},"litellm_metadata":{"session_id":"s1"}}"#,
    );
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn metadata_null_session_id_falls_through_to_litellm_metadata() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(
        r#"{"model":"gpt-4o","metadata":{"session_id":null},"litellm_metadata":{"session_id":"s1"}}"#,
    );
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn metadata_empty_session_id_is_tracked_as_given() {
    let cache = FakeCache::new();
    cache.preload_memory("{session_budget:}:spend", amount("1.0"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"gpt-4o","metadata":{"session_id":""}}"#);
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn metadata_non_object_bucket_falls_through_to_litellm_metadata() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data =
        json(r#"{"model":"gpt-4o","metadata":"none","litellm_metadata":{"session_id":"s1"}}"#);
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn metadata_non_text_session_id_falls_through_to_litellm_metadata() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(
        r#"{"model":"gpt-4o","metadata":{"session_id":5},"litellm_metadata":{"session_id":"s1"}}"#,
    );
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn metadata_non_text_session_id_is_a_no_op() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"gpt-4o","metadata":{"session_id":5}}"#);
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn admission_allows_a_session_whose_local_spend_is_unreadable() {
    let cache = FakeCache::new();
    cache.preload_memory_unreadable(SPEND_KEY);
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_ONE]);
    assert!(
        check(&handler, &registry, Some("agent-1"), &json(REQUEST))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn metadata_absent_session_id_is_a_no_op() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"gpt-4o"}"#);
    assert!(
        check(&handler, &registry, Some("agent-1"), &data)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn error_carries_the_resolved_model_and_provider() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    let rejection = rejection(error);
    assert_eq!(rejection.model, "gpt-4o");
    assert_eq!(rejection.llm_provider, "openai");
    assert_eq!(rejection.category, RateLimitCategory::LitellmRateLimit);
    assert_eq!(rejection.rate_limit_type, Some(RateLimitDimension::Budget));
}

#[tokio::test]
async fn error_without_a_model_uses_the_proxy_fallback() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"metadata":{"session_id":"s1"}}"#);
    let error = check(&handler, &registry, Some("agent-1"), &data)
        .await
        .unwrap_err();
    let rejection = rejection(error);
    assert_eq!(rejection.model, "");
    assert_eq!(rejection.llm_provider, "litellm_proxy");
}

#[tokio::test]
async fn error_keeps_an_unresolvable_model_with_the_fallback_provider() {
    let handler = handler(over_budget_cache());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"mystery","metadata":{"session_id":"s1"}}"#);
    let error = check(&handler, &registry, Some("agent-1"), &data)
        .await
        .unwrap_err();
    let rejection = rejection(error);
    assert_eq!(rejection.model, "mystery");
    assert_eq!(rejection.llm_provider, "litellm_proxy");
}

#[tokio::test]
async fn error_detail_names_the_session_and_the_amounts() {
    let cache = FakeCache::new();
    cache.preload_memory(SPEND_KEY, amount("1.5"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    let Payload::String(detail) = rejection(error).detail else {
        panic!("the detail is text");
    };
    assert_eq!(
        detail,
        "Session budget exceeded for session s1. Current spend: $1.5000, max_budget_per_session: $1.00."
    );
}

#[tokio::test]
async fn error_detail_rounds_decimal_money_half_to_even() {
    let cache = FakeCache::new();
    cache.preload_memory(SPEND_KEY, amount("3.0"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_ROUNDING]);
    let error = check(&handler, &registry, Some("agent-1"), &json(REQUEST))
        .await
        .unwrap_err();
    let Payload::String(detail) = rejection(error).detail else {
        panic!("the detail is text");
    };
    assert_eq!(
        detail,
        "Session budget exceeded for session s1. Current spend: $3.0000, max_budget_per_session: $2.68."
    );
}
