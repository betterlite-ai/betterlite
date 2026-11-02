#[cfg(test)]
use std::time::Instant;

#[cfg(test)]
use secrecy::ExposeSecret as _;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::cache_operations::CacheEntry;
#[cfg(test)]
use super::{MockConfig, MockRegistry, MockToolRegistry, block_on, test_object};

#[cfg(test)]
fn params() -> SandboxToolParams {
    SandboxToolParams {
        sandbox_provider: "e2b".to_owned(),
        api_key: None,
        api_base: None,
    }
}

#[cfg(test)]
#[test]
fn create_container_calls_sandbox_config_when_present() {
    let config = MockConfig::succeeding(test_object([("id", json!("container-1"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider = SandboxProvider {
        sandbox_config: Some(&config),
        registry: &registry,
        tool_registry: &tool_registry,
        sandbox_tool_name: None,
    };
    let (container, used_params) = block_on(provider.create_container()).unwrap();
    assert_eq!(container, test_object([("id", json!("container-1"))]));
    assert!(used_params.is_none());
}

#[cfg(test)]
fn assert_resolved_e2b_params(params: &SandboxToolParams) {
    assert_eq!(params.sandbox_provider, "e2b");
    assert_eq!(
        params.api_key.as_ref().unwrap().expose_secret(),
        "secret-value"
    );
    assert_eq!(params.api_base.as_deref(), Some("https://sandbox.example"));
}

#[cfg(test)]
#[test]
fn create_container_falls_back_to_registry_with_resolved_params() {
    let registry = MockRegistry::succeeding(test_object([("id", json!("container-2"))]));
    let tool_registry = MockToolRegistry {
        resolved: Some(
            test_object([
                ("sandbox_provider", json!("e2b")),
                ("api_key", json!("secret-value")),
                ("api_base", json!("https://sandbox.example")),
            ])
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
    let (container, used_params) = block_on(provider.create_container()).unwrap();
    assert_eq!(container, test_object([("id", json!("container-2"))]));
    assert_resolved_e2b_params(&used_params.unwrap());
    let create_calls = registry.calls.create.lock().unwrap();
    assert_eq!(create_calls.len(), 1);
    assert_resolved_e2b_params(&create_calls[0]);
}

#[cfg(test)]
#[test]
fn create_container_fails_when_config_and_resolved_params_are_both_missing() {
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider: SandboxProvider<'_, MockConfig, MockRegistry, MockToolRegistry> =
        SandboxProvider {
            sandbox_config: None,
            registry: &registry,
            tool_registry: &tool_registry,
            sandbox_tool_name: None,
        };
    let error = block_on(provider.create_container()).unwrap_err();
    assert!(matches!(error, LifecycleError::NoSandboxAvailable));
}

#[cfg(test)]
#[test]
fn run_code_calls_sandbox_config_when_present() {
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
    let result = block_on(provider.run_code(None, &container, "print(1)")).unwrap();
    assert_eq!(result, test_object([("stdout", json!("hi"))]));
    let run_calls = config.calls.run.lock().unwrap();
    assert_eq!(run_calls.as_slice(), [(container, "print(1)".to_owned())]);
}

#[cfg(test)]
#[test]
fn run_code_falls_back_to_registry_with_params() {
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
    let result = block_on(provider.run_code(Some(&params()), &container, "print(1)")).unwrap();
    assert_eq!(result, test_object([("stdout", json!("hi"))]));
    let run_calls = registry.calls.run.lock().unwrap();
    assert_eq!(run_calls.len(), 1);
    assert_eq!(run_calls[0].0.sandbox_provider, params().sandbox_provider);
    assert_eq!(run_calls[0].1, container);
    assert_eq!(run_calls[0].2, "print(1)");
}

#[cfg(test)]
#[test]
fn run_code_fails_when_config_and_params_are_both_missing() {
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
    let error = block_on(provider.run_code(None, &container, "print(1)")).unwrap_err();
    assert!(matches!(error, LifecycleError::NoSandboxAvailable));
}

#[cfg(test)]
#[test]
fn delete_container_calls_sandbox_config_and_swallows_errors() {
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
    block_on(provider.delete_container(None, &container));
    assert_eq!(config.calls.delete.lock().unwrap().as_slice(), [container]);
}

#[cfg(test)]
#[test]
fn delete_container_calls_sandbox_config_when_it_succeeds() {
    let config = MockConfig::succeeding(test_object([("id", json!("container-1"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let provider = SandboxProvider {
        sandbox_config: Some(&config),
        registry: &registry,
        tool_registry: &tool_registry,
        sandbox_tool_name: None,
    };
    let container = test_object([("id", json!("container-1"))]);
    block_on(provider.delete_container(None, &container));
    assert_eq!(config.calls.delete.lock().unwrap().as_slice(), [container]);
}

#[cfg(test)]
#[test]
fn delete_container_falls_back_to_registry_when_config_is_none() {
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
    block_on(provider.delete_container(Some(&params()), &container));
    assert_eq!(registry.calls.delete.lock().unwrap().len(), 1);
}

#[cfg(test)]
#[test]
fn delete_container_does_nothing_when_config_and_params_are_both_missing() {
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
    block_on(provider.delete_container(None, &container));
    assert_eq!(registry.calls.delete.lock().unwrap().len(), 0);
}

#[cfg(test)]
#[test]
fn delete_by_key_removes_the_entry_and_deletes_its_container() {
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
    block_on(provider.delete_container_for_cache_key(&mut cache, Some("cache-key")));
    assert_eq!(cache.entries.len(), 0);
    assert_eq!(registry.calls.delete.lock().unwrap().len(), 1);
}

#[cfg(test)]
#[test]
fn delete_by_key_does_nothing_when_cache_key_is_none() {
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
    block_on(provider.delete_container_for_cache_key(&mut cache, None));
    assert_eq!(registry.calls.delete.lock().unwrap().len(), 0);
}

#[cfg(test)]
#[test]
fn delete_by_key_does_nothing_when_the_cache_key_is_not_cached() {
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
    block_on(provider.delete_container_for_cache_key(&mut cache, Some("missing-key")));
    assert_eq!(registry.calls.delete.lock().unwrap().len(), 0);
}
