#![cfg(test)]

use std::time::Duration;

use super::support::BUDGET_ONE;
use super::support::FakeCache;
use super::support::agent_registry_with;
use super::support::amount;
use super::support::handler;
use super::support::json;
use super::support::success;
use crate::session_budget::INCREMENT_SPEND_SCRIPT;

#[test]
fn structure_handler_with_redis_registers_the_litellm_script_once() {
    let cache = FakeCache::with_redis();
    let handler = handler(cache);
    assert!(handler.script.is_some());
    assert_eq!(handler.ttl, Duration::from_secs(3600));
    let registered = handler.cache.registered.lock().unwrap();
    assert_eq!(registered.len(), 1);
    assert_eq!(
        registered.first().map(String::as_str),
        Some(INCREMENT_SPEND_SCRIPT)
    );
}

#[test]
fn structure_handler_without_redis_registers_no_script() {
    let handler = handler(FakeCache::new());
    assert!(handler.script.is_none());
    assert!(handler.cache.registered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn structure_handler_serves_the_pre_call_and_success_callbacks() {
    let handler = handler(FakeCache::new());
    let registry = agent_registry_with(&[BUDGET_ONE]);
    let data = json(r#"{"model":"gpt-4o","metadata":{"session_id":"s1"}}"#);
    assert!(
        handler
            .async_pre_call_hook(&registry, None, Some("agent-1"), &data)
            .await
            .is_ok()
    );
    let params = json(r#"{"metadata":{"session_id":"s1","agent_id":"agent-1"}}"#);
    handler
        .async_log_success_event(&registry, &success(Some(&params), Some(amount("0.25"))))
        .await;
    assert_eq!(
        handler.cache.memory("{session_budget:s1}:spend"),
        Some(amount("0.25"))
    );
}

#[test]
fn lua_script_is_litellms_existing_incrbyfloat_and_first_write_expire_script() {
    assert_eq!(
        INCREMENT_SPEND_SCRIPT,
        "local key = KEYS[1] local amount = ARGV[1] local ttl = tonumber(ARGV[2]) local existed = redis.call('EXISTS', key) local new_val = redis.call('INCRBYFLOAT', key, amount) if existed == 0 then redis.call('EXPIRE', key, ttl) end return new_val"
    );
}
