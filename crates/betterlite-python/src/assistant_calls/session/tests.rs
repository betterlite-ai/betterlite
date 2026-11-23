#![cfg(test)]

use betterlite_config::sections::resource_sdk_settings::Settings;
use secrecy::ExposeSecret as _;

use super::{ModuleSettings, apply_module_settings};

#[test]
fn module_settings_apply_every_global() {
    let module = ModuleSettings {
        api_key: Some(String::from("sk-a")),
        api_base: Some(String::from("http://base")),
        api_version: Some(String::from("v1")),
        organization: Some(String::from("org")),
        openai_key: Some(String::from("sk-b")),
        azure_key: Some(String::from("az")),
    };
    let mut settings = Settings::default();
    apply_module_settings(&mut settings, &module);
    let key = settings
        .api_key
        .as_ref()
        .map(|secret| secret.expose_secret());
    let openai = settings
        .openai_key
        .as_ref()
        .map(|secret| secret.expose_secret());
    let azure = settings
        .azure_key
        .as_ref()
        .map(|secret| secret.expose_secret());
    assert_eq!(key, Some("sk-a"));
    assert_eq!(settings.api_base.as_deref(), Some("http://base"));
    assert_eq!(settings.api_version.as_deref(), Some("v1"));
    assert_eq!(settings.organization.as_deref(), Some("org"));
    assert_eq!(openai, Some("sk-b"));
    assert_eq!(azure, Some("az"));
}
