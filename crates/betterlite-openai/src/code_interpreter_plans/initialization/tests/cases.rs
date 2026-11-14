#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use betterlite_models::catalog::ModelCatalog;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::{MockConfig, test_object};

#[cfg(test)]
#[test]
fn init_stores_every_setting_and_the_sandbox_config() {
    let config = MockConfig::succeeding(test_object([("id", json!("cnt-1"))]), "");
    let catalog = Arc::new(ModelCatalog::default());
    let logger = CodeInterpreterInterceptionLogger::new(
        false,
        Some(vec!["openai".to_owned()]),
        Some("my-e2b".to_owned()),
        Some(config),
        Arc::clone(&catalog),
    );
    assert!(!logger.enabled);
    assert_eq!(logger.enabled_providers, Some(vec!["openai".to_owned()]));
    assert_eq!(logger.sandbox_tool_name.as_deref(), Some("my-e2b"));
    assert!(logger.sandbox_config.is_some());
    assert!(Arc::ptr_eq(&logger.catalog, &catalog));
    assert!(logger.cache.entries.is_empty());
}

#[cfg(test)]
#[test]
fn init_keeps_absent_settings_as_none() {
    let logger = CodeInterpreterInterceptionLogger::<MockConfig>::new(
        true,
        None,
        None,
        None,
        Arc::new(ModelCatalog::default()),
    );
    assert!(logger.enabled);
    assert!(logger.enabled_providers.is_none());
    assert!(logger.sandbox_tool_name.is_none());
    assert!(logger.sandbox_config.is_none());
    assert!(logger.cache.entries.is_empty());
}
