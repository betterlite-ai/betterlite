#[cfg(test)]
use serde_json::json;

#[cfg(test)]
fn config(value: serde_json::Value) -> CodeInterpreterInterceptionConfig {
    serde_json::from_value(value).unwrap()
}

#[cfg(test)]
#[test]
fn from_yaml_reads_the_enabled_flag_providers_and_sandbox_tool_name() {
    let parsed = config(json!({
        "enabled": false,
        "enabled_providers": ["openai"],
        "sandbox_tool_name": "my-e2b"
    }));
    let logger = from_config_yaml::<()>(&parsed);
    assert!(!logger.enabled);
    assert_eq!(logger.enabled_providers, Some(vec!["openai".to_owned()]));
    assert_eq!(logger.sandbox_tool_name.as_deref(), Some("my-e2b"));
    assert!(logger.sandbox_config.is_none());
    assert!(logger.cache.entries.is_empty());
}

#[cfg(test)]
#[test]
fn from_yaml_defaults_enabled_to_true_and_leaves_the_optional_settings_absent() {
    let parsed = config(json!({"enabled_providers": null}));
    let logger = from_config_yaml::<()>(&parsed);
    assert!(logger.enabled);
    assert!(logger.enabled_providers.is_none());
    assert!(logger.sandbox_tool_name.is_none());
}

#[cfg(test)]
#[test]
fn from_proxy_prefers_the_settings_entry_over_callback_parameters() {
    let settings = config(json!({"enabled": false, "enabled_providers": ["azure"]}));
    let callback = config(json!({"enabled": true, "enabled_providers": ["openai"]}));
    let logger = initialize_from_proxy_config::<()>(&ProxyParameters {
        code_interpreter_interception_params: Some(settings),
        callback_specific_params: Some(callback),
    });
    assert!(!logger.enabled);
    assert_eq!(logger.enabled_providers, Some(vec!["azure".to_owned()]));
}

#[cfg(test)]
#[test]
fn from_proxy_falls_back_to_callback_parameters() {
    let callback = config(json!({"sandbox_tool_name": "my-e2b"}));
    let logger = initialize_from_proxy_config::<()>(&ProxyParameters {
        code_interpreter_interception_params: None,
        callback_specific_params: Some(callback),
    });
    assert!(logger.enabled);
    assert_eq!(logger.sandbox_tool_name.as_deref(), Some("my-e2b"));
}

#[cfg(test)]
#[test]
fn from_proxy_falls_back_to_the_default_config_when_both_are_absent() {
    let logger = initialize_from_proxy_config::<()>(&ProxyParameters::default());
    assert!(logger.enabled);
    assert!(logger.enabled_providers.is_none());
    assert!(logger.sandbox_tool_name.is_none());
}
