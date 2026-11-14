#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::sandbox_dispatch::{SandboxToolConfig, SandboxToolEntry};

use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn load(source: &str) -> Result<Config, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
}

fn entries(config: &Config) -> &[SandboxToolEntry] {
    &config.sections.sandbox_dispatch.sandbox_tools
}

#[test]
fn loads_the_configured_sandbox_tools() {
    let config = load(
        "sandbox_tools:\n  - sandbox_tool_name: my-e2b\n    litellm_params:\n      sandbox_provider: e2b\n      api_key: os.environ/E2B_API_KEY\n  - sandbox_tool_name: my-open\n    litellm_params:\n      sandbox_provider: opensandbox\n      api_base: https://opensandbox.test\n",
    )
    .unwrap();
    assert_eq!(
        entries(&config).to_vec(),
        vec![
            SandboxToolEntry::Configured(SandboxToolConfig {
                name: Some("my-e2b".to_owned()),
                provider: Some("e2b".to_owned()),
                api_key: Some("os.environ/E2B_API_KEY".to_owned()),
                api_base: None,
            }),
            SandboxToolEntry::Configured(SandboxToolConfig {
                name: Some("my-open".to_owned()),
                provider: Some("opensandbox".to_owned()),
                api_key: None,
                api_base: Some("https://opensandbox.test".to_owned()),
            }),
        ]
    );
}

#[test]
fn a_secret_reference_stays_as_written_until_the_registry_reads_it() {
    let config =
        load("sandbox_tools:\n  - sandbox_tool_name: t\n    litellm_params:\n      sandbox_provider: e2b\n      api_key: os.environ/MY_KEY\n")
            .unwrap();
    let [SandboxToolEntry::Configured(entry)] = entries(&config) else {
        panic!("one configured entry");
    };
    assert_eq!(entry.api_key.as_deref(), Some("os.environ/MY_KEY"));
}

#[test]
fn absent_sandbox_tools_reads_as_no_entries() {
    let config = load("model_list: []\n").unwrap();
    assert!(entries(&config).is_empty());
}

#[test]
fn null_sandbox_tools_reads_as_no_entries() {
    let config = load("sandbox_tools:\n").unwrap();
    assert!(entries(&config).is_empty());
}

#[test]
fn an_entry_that_is_not_a_mapping_is_kept_for_the_registry_to_skip() {
    let config = load("sandbox_tools:\n  - not-a-tool\n  - 5\n").unwrap();
    assert_eq!(
        entries(&config).to_vec(),
        vec![SandboxToolEntry::NotAnObject, SandboxToolEntry::NotAnObject]
    );
}

#[test]
fn entries_missing_a_name_or_provider_are_kept_for_the_registry_to_skip() {
    let config = load(
        "sandbox_tools:\n  - litellm_params:\n      sandbox_provider: e2b\n  - sandbox_tool_name: no-provider\n",
    )
    .unwrap();
    assert_eq!(
        entries(&config).to_vec(),
        vec![
            SandboxToolEntry::Configured(SandboxToolConfig {
                name: None,
                provider: Some("e2b".to_owned()),
                api_key: None,
                api_base: None,
            }),
            SandboxToolEntry::Configured(SandboxToolConfig {
                name: Some("no-provider".to_owned()),
                provider: None,
                api_key: None,
                api_base: None,
            }),
        ]
    );
}

#[test]
fn a_name_that_is_not_text_is_an_error_naming_its_path() {
    let error = load("sandbox_tools:\n  - sandbox_tool_name: 5\n").unwrap_err();
    assert!(matches!(error, ConfigError::InvalidScalar { .. }));
    assert!(
        error
            .to_string()
            .contains("sandbox_tools[0].sandbox_tool_name")
    );
}

#[test]
fn litellm_params_that_is_not_a_mapping_is_an_error_naming_its_path() {
    let error =
        load("sandbox_tools:\n  - sandbox_tool_name: t\n    litellm_params: e2b\n").unwrap_err();
    assert!(matches!(error, ConfigError::InvalidScalar { .. }));
    assert!(
        error
            .to_string()
            .contains("sandbox_tools[0].litellm_params")
    );
}

#[test]
fn sandbox_tools_that_is_not_a_list_is_a_config_error() {
    let error = load("sandbox_tools:\n  my-e2b:\n    sandbox_provider: e2b\n").unwrap_err();
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[test]
fn keys_this_section_does_not_read_stay_unsupported() {
    let error = load(
        "sandbox_tools: []\nlitellm_settings:\n  code_interpreter_interception_params:\n    enabled_providers: [openai]\n",
    )
    .unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unread key is unsupported");
    };
    assert_eq!(
        paths,
        ["litellm_settings.code_interpreter_interception_params"]
    );
}
