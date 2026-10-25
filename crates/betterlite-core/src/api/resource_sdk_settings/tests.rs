#![cfg(test)]

use super::{
    FineTuningTimeout, request_timeout, resolve_fine_tuning_timeout, supports_httpx_timeout,
};

#[test]
fn timeout_capability_and_conversion() {
    assert!(supports_httpx_timeout("openai"));
    assert!(!supports_httpx_timeout("vertex_ai"));
    assert!((request_timeout(Some(0.0_f64), Some(9.0_f64)) - 9.0_f64).abs() < f64::EPSILON);
    assert!((request_timeout(Some(0.0_f64), Some(0.0_f64)) - 600.0_f64).abs() < f64::EPSILON);
    assert_eq!(
        resolve_fine_tuning_timeout(Some("1_000"), "openai", None),
        Ok(FineTuningTimeout::Seconds(1000.0))
    );
}

#[test]
fn settings_debug_redacts_secret_fields() {
    let mut settings = test_settings();
    settings.api_key = Some(secrecy::SecretString::from("secret"));
    assert!(!format!("{settings:?}").contains("secret"));
}

fn test_settings() -> super::ResourceSdkSettings {
    super::ResourceSdkSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}
