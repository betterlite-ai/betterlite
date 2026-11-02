use std::time::Instant;

use serde_json::json;

use super::super::cache_operations::CacheEntry;
use super::super::{MockConfig, MockRegistry, MockToolRegistry, test_object};

fn params() -> SandboxToolParams {
    SandboxToolParams {
        sandbox_provider: "e2b".to_owned(),
        api_key: None,
        api_base: None,
    }
}

#[tokio::test]
async fn create_container_calls_sandbox_config_when_present() {
    let config = MockConfig::succeeding(test_object([("id", json!("container-1"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider = SandboxProvider {
        sandbox_config: Some(&config),
        registry: &registry,
        tool_registry: &tool_registry,
        sandbox_tool_name: None,
    };
    let (container, used_params) = provider.create_container().await.unwrap();
    assert_eq!(container, test_object([("id", json!("container-1"))]));
    assert!(used_params.is_none());
}

#[tokio::test]
async fn create_container_falls_back_to_registry_with_resolved_params() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("container-2"))]));
    let tool_registry = MockToolRegistry {
        resolved: Some(
            test_object([("sandbox_provider", json!("e2b"))])
                .as_object()
                .unwrap()
                .clone(),
        ),
    };
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: Some("my-tool"),
        };
    let (container, used_params) = provider.create_container().await.unwrap();
    assert_eq!(container, test_object([("id", json!("container-2"))]));
    let resolved_params = used_params.unwrap();
    assert_eq!(resolved_params.sandbox_provider, params().sandbox_provider);
    assert!(resolved_params.api_key.is_none());
    assert_eq!(resolved_params.api_base, params().api_base);
}

#[tokio::test]
async fn create_container_fails_when_config_and_resolved_params_are_both_missing() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let error = provider.create_container().await.unwrap_err();
    assert!(matches!(error, LifecycleError::NoSandboxAvailable));
}

#[tokio::test]
async fn run_code_calls_sandbox_config_when_present() {
    let config = MockConfig::succeeding(test_object([("stdout", json!("hi"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider = SandboxProvider {
        sandbox_config: Some(&config),
        registry: &registry,
        tool_registry: &tool_registry,
        sandbox_tool_name: None,
    };
    let container = test_object([("id", json!("container-1"))]);
    let result = provider
        .run_code(None, &container, "print(1)")
        .await
        .unwrap();
    assert_eq!(result, test_object([("stdout", json!("hi"))]));
}

#[tokio::test]
async fn run_code_falls_back_to_registry_with_params() {
    let registry = MockRegistry::succeeding(test_object([("stdout", json!("hi"))]));
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let container = test_object([("id", json!("container-2"))]);
    let result = provider
        .run_code(Some(&params()), &container, "print(1)")
        .await
        .unwrap();
    assert_eq!(result, test_object([("stdout", json!("hi"))]));
}

#[tokio::test]
async fn run_code_fails_when_config_and_params_are_both_missing() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let container = test_object([("id", json!("container-1"))]);
    let error = provider
        .run_code(None, &container, "print(1)")
        .await
        .unwrap_err();
    assert!(matches!(error, LifecycleError::NoSandboxAvailable));
}

#[tokio::test]
async fn delete_container_calls_sandbox_config_and_swallows_errors() {
    let config = MockConfig::failing("boom");
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider = SandboxProvider {
        sandbox_config: Some(&config),
        registry: &registry,
        tool_registry: &tool_registry,
        sandbox_tool_name: None,
    };
    let container = test_object([("id", json!("container-1"))]);
    provider.delete_container(None, &container).await;
}

#[tokio::test]
async fn delete_container_falls_back_to_registry_when_config_is_none() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("container-2"))]));
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let container = test_object([("id", json!("container-2"))]);
    provider.delete_container(Some(&params()), &container).await;
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delete_container_does_nothing_when_config_and_params_are_both_missing() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let container = test_object([("id", json!("container-1"))]);
    provider.delete_container(None, &container).await;
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn delete_by_key_removes_the_entry_and_deletes_its_container() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("container-3"))]));
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let mut cache = SandboxCache::default();
    cache.entries.insert(
        "cache-key".to_owned(),
        CacheEntry {
            container: test_object([("id", json!("container-3"))]),
            params: Some(params()),
            last_accessed: Instant::now(),
            identity: None,
        },
    );
    provider
        .delete_container_for_cache_key(&mut cache, Some("cache-key"))
        .await;
    assert_eq!(cache.entries.len(), 0);
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delete_by_key_does_nothing_when_cache_key_is_none() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let mut cache = SandboxCache::default();
    provider
        .delete_container_for_cache_key(&mut cache, None)
        .await;
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn delete_by_key_does_nothing_when_the_cache_key_is_not_cached() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let mut cache = SandboxCache::default();
    provider
        .delete_container_for_cache_key(&mut cache, Some("missing-key"))
        .await;
    assert_eq!(registry.delete_calls.lock().unwrap().len(), 0);
}
