#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::{MockConfig, test_object};

#[cfg(test)]
#[test]
fn init_stores_every_setting_and_the_sandbox_config() {
    let config = MockConfig::succeeding(test_object([("id", json!("cnt-1"))]), "");
    let logger = CodeInterpreterInterceptionLogger {
        enabled: false,
        enabled_providers: Some(vec!["openai".to_owned()]),
        sandbox_tool_name: Some("my-e2b".to_owned()),
        sandbox_config: Some(config),
        cache: SandboxCache::default(),
    };
    assert!(!logger.enabled);
    assert_eq!(logger.enabled_providers, Some(vec!["openai".to_owned()]));
    assert_eq!(logger.sandbox_tool_name.as_deref(), Some("my-e2b"));
    assert!(logger.sandbox_config.is_some());
    assert!(logger.cache.entries.is_empty());
}

#[cfg(test)]
#[test]
fn init_keeps_absent_settings_as_none() {
    let logger = CodeInterpreterInterceptionLogger::<MockConfig> {
        enabled: true,
        enabled_providers: None,
        sandbox_tool_name: None,
        sandbox_config: None,
        cache: SandboxCache::default(),
    };
    assert!(logger.enabled);
    assert!(logger.enabled_providers.is_none());
    assert!(logger.sandbox_tool_name.is_none());
    assert!(logger.sandbox_config.is_none());
}
