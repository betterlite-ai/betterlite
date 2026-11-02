use std::time::Instant;

use serde_json::json;

use super::super::cache_operations::CacheEntry;
use super::super::resolution::SandboxToolParams;
use super::super::{MockConfig, MockRegistry, MockToolRegistry, test_object};

fn provider<'provider>(
    registry: &'provider MockRegistry,
    tool_registry: &'provider MockToolRegistry,
) -> SandboxProvider<'provider, MockConfig, MockRegistry, MockToolRegistry> {
    SandboxProvider {
        sandbox_config: None,
        registry,
        tool_registry,
        sandbox_tool_name: None,
    }
}

fn cached(cache: &mut SandboxCache, key: &str) {
    cache.entries.insert(
        key.to_owned(),
        CacheEntry {
            container: test_object([("id", json!(key))]),
            params: Some(SandboxToolParams {
                sandbox_provider: "e2b".to_owned(),
                api_key: None,
                api_base: None,
            }),
            last_accessed: Instant::now(),
            identity: None,
        },
    );
}

#[tokio::test]
async fn cleanup_leaves_session_scoped_containers_in_the_cache() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("cache-key"))]));
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&registry, &tool_registry);
    let mut cache = SandboxCache::default();
    cached(&mut cache, "cache-key");
    let metadata = PlanMetadata {
        is_session_scoped: true,
        sandbox_key: Some("cache-key".to_owned()),
        code_interpreter_calls: Vec::new(),
    };
    cleanup_hook(&sandbox, &mut cache, &metadata).await;
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn cleanup_deletes_request_scoped_containers() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("cache-key"))]));
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&registry, &tool_registry);
    let mut cache = SandboxCache::default();
    cached(&mut cache, "cache-key");
    let metadata = PlanMetadata {
        is_session_scoped: false,
        sandbox_key: Some("cache-key".to_owned()),
        code_interpreter_calls: Vec::new(),
    };
    cleanup_hook(&sandbox, &mut cache, &metadata).await;
    assert_eq!(cache.entries.len(), 0);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cleanup_does_nothing_when_the_sandbox_key_is_absent() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let metadata = PlanMetadata {
        is_session_scoped: false,
        sandbox_key: None,
        code_interpreter_calls: Vec::new(),
    };
    cleanup_hook(&sandbox, &mut cache, &metadata).await;
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn post_response_merges_code_interpreter_calls_before_the_first_message_item() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let call = test_object([
        ("type", json!("code_interpreter_call")),
        ("id", json!("ci_1")),
    ]);
    let metadata = PlanMetadata {
        is_session_scoped: true,
        sandbox_key: None,
        code_interpreter_calls: vec![call.clone()],
    };
    let function_call = test_object([
        ("type", json!("function_call")),
        ("name", json!("litellm_code_execution")),
    ]);
    let message = test_object([("type", json!("message")), ("content", json!("hi"))]);
    let response = test_object([(
        "output",
        Value::Array(vec![function_call.clone(), message.clone()]),
    )]);
    let merged = post_response_hook(&sandbox, &mut cache, &metadata, response).await;
    assert_eq!(
        merged,
        test_object([("output", Value::Array(vec![function_call, call, message]))])
    );
}

#[tokio::test]
async fn post_response_returns_the_response_unchanged_when_there_are_no_calls() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&registry, &tool_registry);
    let mut cache = SandboxCache::default();
    let metadata = PlanMetadata {
        is_session_scoped: true,
        sandbox_key: None,
        code_interpreter_calls: Vec::new(),
    };
    let message = test_object([("type", json!("message")), ("content", json!("hi"))]);
    let response = test_object([("output", Value::Array(vec![message]))]);
    let unchanged = post_response_hook(&sandbox, &mut cache, &metadata, response.clone()).await;
    assert_eq!(unchanged, response);
}

#[tokio::test]
async fn post_response_leaves_session_scoped_containers_in_the_cache() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("cache-key"))]));
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&registry, &tool_registry);
    let mut cache = SandboxCache::default();
    cached(&mut cache, "cache-key");
    let metadata = PlanMetadata {
        is_session_scoped: true,
        sandbox_key: Some("cache-key".to_owned()),
        code_interpreter_calls: Vec::new(),
    };
    let response = test_object([("output", Value::Array(Vec::new()))]);
    post_response_hook(&sandbox, &mut cache, &metadata, response).await;
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn post_response_deletes_request_scoped_containers() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("cache-key"))]));
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&registry, &tool_registry);
    let mut cache = SandboxCache::default();
    cached(&mut cache, "cache-key");
    let metadata = PlanMetadata {
        is_session_scoped: false,
        sandbox_key: Some("cache-key".to_owned()),
        code_interpreter_calls: Vec::new(),
    };
    let response = test_object([("output", Value::Array(Vec::new()))]);
    post_response_hook(&sandbox, &mut cache, &metadata, response).await;
    assert_eq!(cache.entries.len(), 0);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 1);
}
