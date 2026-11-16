#![cfg(test)]

use std::time::Duration;

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::session_budget::tests::support::BUDGET_INVALID;
use crate::session_budget::tests::support::BUDGET_NULL;
use crate::session_budget::tests::support::BUDGET_ONE;
use crate::session_budget::tests::support::BUDGET_TEXT;
use crate::session_budget::tests::support::FakeCache;
use crate::session_budget::tests::support::NO_BUDGET;
use crate::session_budget::tests::support::agent;
use crate::session_budget::tests::support::agent_registry_with;
use crate::session_budget::tests::support::amount;
use crate::session_budget::tests::support::handler;
use crate::session_budget::tests::support::handler_at;
use crate::session_budget::tests::support::json;
use crate::session_budget::tests::support::success;

const KEY: &str = "{session_budget:s1}:spend";
const PARAMS: &str = r#"{"metadata":{"session_id":"s1","agent_id":"agent-1"}}"#;

#[tokio::test]
async fn charging_positive_cost_adds_to_the_session_spend() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let event = success(Some(&json(PARAMS)), Some(amount("0.25")));
    handler.async_log_success_event(&registry, &event).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
}

#[tokio::test]
async fn charging_accumulates_across_successful_calls() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(PARAMS);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.5")));
}

#[tokio::test]
async fn charging_zero_cost_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    handler
        .async_log_success_event(&registry, &success(Some(&json(PARAMS)), Some(amount("0"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_negative_cost_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    handler
        .async_log_success_event(
            &registry,
            &success(Some(&json(PARAMS)), Some(amount("-0.25"))),
        )
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_absent_cost_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    handler
        .async_log_success_event(&registry, &success(Some(&json(PARAMS)), None))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_without_litellm_params_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    handler
        .async_log_success_event(&registry, &success(None, Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_without_metadata_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"model":"gpt-4o"}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_with_non_object_metadata_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":"none"}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_without_a_session_id_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":{"agent_id":"agent-1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_without_an_agent_id_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":{"session_id":"s1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_non_text_session_id_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":{"session_id":5,"agent_id":"agent-1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_non_text_agent_id_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":{"session_id":"s1","agent_id":5}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_unknown_agent_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":{"session_id":"s1","agent_id":"agent-9"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_agent_without_the_budget_key_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[NO_BUDGET]);
    let params = json(r#"{"metadata":{"session_id":"s1","agent_id":"agent-2"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_null_budget_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_NULL]);
    handler
        .async_log_success_event(
            &registry,
            &success(Some(&json(PARAMS)), Some(amount("0.25"))),
        )
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_agent_whose_budget_text_is_not_a_number_is_charged() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_INVALID]);
    handler
        .async_log_success_event(
            &registry,
            &success(Some(&json(PARAMS)), Some(amount("0.25"))),
        )
        .await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
}

#[tokio::test]
async fn charging_session_id_only_in_litellm_metadata_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params =
        json(r#"{"metadata":{"agent_id":"agent-1"},"litellm_metadata":{"session_id":"s1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_session_id_nested_under_metadata_is_skipped() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params =
        json(r#"{"metadata":{"litellm_metadata":{"session_id":"s1"},"agent_id":"agent-1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(handler.cache.memory(KEY), None);
}

#[tokio::test]
async fn charging_agent_registry_lookup_is_live_at_success_time() {
    let handler = handler(FakeCache::new());
    let registry = AgentRegistry::default();
    let params = json(PARAMS);
    let event = success(Some(&params), Some(amount("0.25")));
    handler.async_log_success_event(&registry, &event).await;
    assert_eq!(handler.cache.memory(KEY), None);
    registry.register_agent(agent(BUDGET_ONE));
    handler.async_log_success_event(&registry, &event).await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
}

#[tokio::test]
async fn charging_text_budget_is_enough_to_charge() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_TEXT]);
    handler
        .async_log_success_event(
            &registry,
            &success(Some(&json(PARAMS)), Some(amount("0.25"))),
        )
        .await;
    assert_eq!(handler.cache.memory(KEY), Some(amount("0.25")));
}

#[tokio::test]
async fn charging_records_the_configured_ttl() {
    let handler = handler_at(FakeCache::new(), Duration::from_secs(120));
    let registry = agent_registry_with(&[BUDGET_ONE]);
    handler
        .async_log_success_event(
            &registry,
            &success(Some(&json(PARAMS)), Some(amount("0.25"))),
        )
        .await;
    assert_eq!(
        handler.cache.memory_ttl(KEY),
        Some(Duration::from_secs(120))
    );
}
