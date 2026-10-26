#![cfg(test)]

use super::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, request_timeout,
    resolve_fine_tuning_timeout, supports_httpx_timeout,
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
    assert_eq!(
        resolve_fine_tuning_timeout(Some(""), "openai", None),
        Ok(FineTuningTimeout::Seconds(600.0))
    );
}

#[test]
fn structured_timeout_conversion() {
    let structured = StructuredTimeout {
        connect: Some(1.0_f64),
        read: Some(2.0_f64),
        write: Some(3.0_f64),
        pool: Some(4.0_f64),
    };
    assert_eq!(
        resolve_fine_tuning_timeout(None, "openai", Some(structured)),
        Ok(FineTuningTimeout::Structured(structured))
    );
    assert_eq!(
        resolve_fine_tuning_timeout(
            None,
            "vertex_ai",
            Some(StructuredTimeout {
                read: Some(0.0_f64),
                ..structured
            })
        ),
        Ok(FineTuningTimeout::Seconds(600.0))
    );
    assert_eq!(
        resolve_fine_tuning_timeout(
            None,
            "openai",
            Some(StructuredTimeout {
                connect: Some(f64::INFINITY),
                ..structured
            })
        ),
        Err(TimeoutError::NonFinite)
    );
}

#[test]
fn resource_precedence_ignores_empty_values() {
    let mut settings = test_settings();
    settings.api_base = Some(String::new());
    settings.openai_base_url = Some("openai-base".to_owned());
    settings.openai_api_base = Some("legacy-base".to_owned());
    settings.api_key = Some(secrecy::SecretString::from(""));
    settings.openai_key = Some(secrecy::SecretString::from("openai-key"));
    settings.azure_api_version = Some("azure-version".to_owned());
    settings.vertex_location = Some(String::new());
    settings.vertexai_location = Some("vertex-location".to_owned());
    assert_eq!(settings.openai_base(), "openai-base");
    assert_eq!(
        settings
            .openai_api_key()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("openai-key")
    );
    assert_eq!(settings.azure_version(), Some("azure-version"));
    assert_eq!(settings.vertex_location(), Some("vertex-location"));
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
