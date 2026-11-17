#![cfg(test)]

use betterlite_error::rate_limit::RateLimitDimension;

use super::support::BUDGET_ONE;
use super::support::FakeCache;
use super::support::agent_registry_with;
use super::support::amount;
use super::support::handler;
use super::support::json;
use super::support::rejection;
use super::support::success;

#[tokio::test]
async fn cache_key_format_is_the_litellm_braced_session_key() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":{"session_id":"s1","agent_id":"agent-1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(
        handler.cache.memory("{session_budget:s1}:spend"),
        Some(amount("0.25"))
    );
}

#[tokio::test]
async fn cache_key_embeds_colons_in_the_session_id_unquoted() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let params = json(r#"{"metadata":{"session_id":"a:b","agent_id":"agent-1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.5"))))
        .await;
    assert_eq!(
        handler.cache.memory("{session_budget:a:b}:spend"),
        Some(amount("0.5"))
    );
}

#[tokio::test]
async fn cache_key_is_what_the_pre_call_read_uses() {
    let cache = FakeCache::new();
    cache.preload_memory("{session_budget:s1}:spend", amount("1.0"));
    let handler = handler(cache);
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"gpt-4o","metadata":{"session_id":"s1"}}"#);
    let error = handler
        .async_pre_call_hook(&registry, None, Some("agent-1"), &data)
        .await
        .unwrap_err();
    assert_eq!(
        rejection(error).rate_limit_type,
        Some(RateLimitDimension::Budget)
    );
}
