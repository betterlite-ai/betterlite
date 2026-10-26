#![cfg(test)]

use super::ResourceSdkRequestOptions;
use super::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout, supports_httpx_timeout,
};
use secrecy::{ExposeSecret as _, SecretString};

#[test]
fn openai_resource_precedence() {
    let mut settings = test_settings();
    settings.api_base = Some("global-base".to_owned());
    settings.openai_base_url = Some("env-base".to_owned());
    settings.api_key = Some(SecretString::from("global-key"));
    settings.openai_api_key = Some(SecretString::from("env-key"));
    settings.organization = Some("global-org".to_owned());
    settings.openai_organization = Some("env-org".to_owned());
    let mut request = request_options();
    request.api_base = Some("request-base".to_owned());
    request.api_key = Some(SecretString::from("request-key"));
    request.organization = Some("request-org".to_owned());
    assert_eq!(settings.openai_base(&request), "request-base");
    assert_eq!(
        exposed(settings.openai_api_key(&request)),
        Some("request-key")
    );
    assert_eq!(settings.openai_organization(&request), Some("request-org"));
    request.api_base = None;
    request.api_key = None;
    request.organization = None;
    assert_eq!(settings.openai_base(&request), "global-base");
    assert_eq!(
        exposed(settings.openai_api_key(&request)),
        Some("global-key")
    );
    assert_eq!(settings.openai_organization(&request), Some("global-org"));
    settings.api_base = None;
    settings.api_key = None;
    settings.organization = None;
    assert_eq!(settings.openai_base(&request), "env-base");
    assert_eq!(exposed(settings.openai_api_key(&request)), Some("env-key"));
    assert_eq!(settings.openai_organization(&request), Some("env-org"));
    request.api_version = Some("request-version".to_owned());
    assert_eq!(
        settings.openai_api_version(&request),
        Some("request-version")
    );
}

#[test]
fn azure_resource_precedence() {
    let mut settings = test_settings();
    settings.api_base = Some("global-base".to_owned());
    settings.azure_api_base = Some("env-base".to_owned());
    settings.api_version = Some("global-version".to_owned());
    settings.azure_api_version = Some("env-version".to_owned());
    settings.api_key = Some(SecretString::from("global-key"));
    settings.azure_openai_api_key = Some(SecretString::from("env-key"));
    let mut request = request_options();
    request.api_base = Some("request-base".to_owned());
    request.api_version = Some("request-version".to_owned());
    request.api_key = Some(SecretString::from("request-key"));
    request.organization = Some("request-org".to_owned());
    assert_eq!(settings.azure_base(&request), Some("request-base"));
    assert_eq!(settings.azure_version(&request), Some("request-version"));
    assert_eq!(
        exposed(settings.azure_api_key(&request)),
        Some("request-key")
    );
    assert_eq!(settings.azure_organization(&request), Some("request-org"));
    request.api_base = None;
    request.api_version = None;
    request.api_key = None;
    request.organization = None;
    assert_eq!(settings.azure_base(&request), Some("global-base"));
    assert_eq!(settings.azure_version(&request), Some("global-version"));
    assert_eq!(
        exposed(settings.azure_api_key(&request)),
        Some("global-key")
    );
    assert_eq!(settings.azure_organization(&request), None);
    settings.api_base = None;
    settings.api_version = None;
    settings.api_key = None;
    assert_eq!(settings.azure_base(&request), Some("env-base"));
    assert_eq!(settings.azure_version(&request), Some("env-version"));
    assert_eq!(exposed(settings.azure_api_key(&request)), Some("env-key"));
}

#[test]
fn vertex_resource_settings() {
    let mut settings = test_settings();
    settings.api_base = Some("global-base-must-be-ignored".to_owned());
    settings.vertex_project = Some("global-project".to_owned());
    settings.vertexai_project = Some("env-project".to_owned());
    settings.vertex_location = Some("global-location".to_owned());
    settings.vertexai_location = Some("env-location".to_owned());
    settings.vertexai_credentials = Some(SecretString::from("env-credentials"));
    let mut request = request_options();
    request.api_base = Some("request-base".to_owned());
    request.vertex_project = Some("request-project".to_owned());
    request.vertex_location = Some("request-location".to_owned());
    request.vertex_credentials = Some(SecretString::from("request-credentials"));
    assert_eq!(settings.vertex_base(&request), "request-base");
    assert_eq!(settings.vertex_project(&request), Some("request-project"));
    assert_eq!(settings.vertex_location(&request), Some("request-location"));
    assert_eq!(
        exposed(settings.vertex_credentials(&request)),
        Some("request-credentials")
    );
    request.api_base = None;
    request.vertex_project = None;
    request.vertex_location = None;
    request.vertex_credentials = None;
    assert_eq!(settings.vertex_base(&request), "");
    assert_eq!(settings.vertex_project(&request), Some("global-project"));
    assert_eq!(settings.vertex_location(&request), Some("global-location"));
    assert_eq!(
        exposed(settings.vertex_credentials(&request)),
        Some("env-credentials")
    );
    settings.vertex_project = None;
    settings.vertex_location = None;
    assert_eq!(settings.vertex_project(&request), Some("env-project"));
    assert_eq!(settings.vertex_location(&request), Some("env-location"));
}

#[test]
fn timeout_capability_and_conversion() {
    assert!(supports_httpx_timeout("openai"));
    assert!(supports_httpx_timeout("azure"));
    assert!(supports_httpx_timeout("bedrock"));
    assert!(!supports_httpx_timeout("vertex_ai"));
    assert_eq!(request_timeout(Some(0.0_f64), Some(9.0_f64)), Ok(9.0_f64));
    assert_eq!(request_timeout(Some(0.0_f64), Some(0.0_f64)), Ok(600.0_f64));
    assert_raw_timeout_values();
    assert_eq!(
        resolve_fine_tuning_timeout(Some(&TimeoutValue::Number(0.0)), "openai", None),
        Ok(FineTuningTimeout::Seconds(600.0))
    );
}

fn assert_raw_timeout_values() {
    assert_eq!(
        resolve_fine_tuning_timeout(
            Some(&TimeoutValue::String("7.5".to_owned())),
            "openai",
            None
        ),
        Ok(FineTuningTimeout::Seconds(7.5_f64))
    );
    assert_eq!(
        resolve_fine_tuning_timeout(
            Some(&TimeoutValue::String(" 7 ".to_owned())),
            "openai",
            None
        ),
        Ok(FineTuningTimeout::Seconds(7.0_f64))
    );
    assert_eq!(
        resolve_fine_tuning_timeout(
            Some(&TimeoutValue::String("1_000".to_owned())),
            "openai",
            None
        ),
        Ok(FineTuningTimeout::Seconds(1000.0_f64))
    );
    assert_invalid_timeout_values();
}

fn assert_invalid_timeout_values() {
    assert_invalid_timeout("bad");
    assert_invalid_timeout("_1");
    assert_invalid_timeout("1__0");
}

fn assert_invalid_timeout(value: &str) {
    assert!(matches!(
        resolve_fine_tuning_timeout(
            Some(&TimeoutValue::String(value.to_owned())),
            "openai",
            None
        ),
        Err(TimeoutError::InvalidNumber { .. })
    ));
}

#[test]
fn structured_timeout_conversion_rejects_non_finite_values() {
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
    assert_eq!(
        request_timeout(Some(f64::NAN), None),
        Err(TimeoutError::NonFinite)
    );
}

#[test]
fn settings_debug_redacts_secret_fields() {
    let mut settings = test_settings();
    settings.api_key = Some(SecretString::from("secret"));
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

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

fn request_options() -> ResourceSdkRequestOptions {
    ResourceSdkRequestOptions {
        api_base: None,
        api_version: None,
        api_key: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
    }
}
