#![cfg(test)]

use super::ResourceSdkRequestOptions;
use super::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout, supports_httpx_timeout,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn openai_resource_precedence(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    request.api_key = Some(SecretString::from("request"));
    settings.api_key = Some(SecretString::from("global"));
    settings.openai_key = Some(SecretString::from("openai"));
    settings.openai_api_key = Some(SecretString::from("env"));
    assert_eq!(exposed(settings.openai_api_key(&request)), Some("request"));
    request.api_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.openai_api_key(&request)), Some("global"));
    settings.api_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.openai_api_key(&request)), Some("openai"));
    settings.openai_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.openai_api_key(&request)), Some("env"));
    settings.openai_api_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.openai_api_key(&request)), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn openai_base_precedence(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    request.api_base = Some("request".to_owned());
    settings.api_base = Some("api_base".to_owned());
    settings.openai_base_url = Some("openai_base_url".to_owned());
    settings.openai_api_base = Some("openai_api_base".to_owned());
    assert_eq!(settings.openai_base(&request), "request");
    request.api_base = cleared.map(str::to_owned);
    assert_eq!(settings.openai_base(&request), "api_base");
    settings.api_base = cleared.map(str::to_owned);
    assert_eq!(settings.openai_base(&request), "openai_base_url");
    settings.openai_base_url = cleared.map(str::to_owned);
    assert_eq!(settings.openai_base(&request), "openai_api_base");
    settings.openai_api_base = cleared.map(str::to_owned);
    assert_eq!(settings.openai_base(&request), "https://api.openai.com/v1");
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn openai_org_precedence(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    request.organization = Some("request".to_owned());
    settings.organization = Some("global-org".to_owned());
    settings.openai_organization = Some("env-org".to_owned());
    assert_eq!(settings.openai_organization(&request), Some("request"));
    request.organization = cleared.map(str::to_owned);
    assert_eq!(settings.openai_organization(&request), Some("global-org"));
    settings.organization = cleared.map(str::to_owned);
    assert_eq!(settings.openai_organization(&request), Some("env-org"));
    settings.openai_organization = cleared.map(str::to_owned);
    assert_eq!(settings.openai_organization(&request), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn azure_resource_precedence(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    request.api_key = Some(SecretString::from("request"));
    settings.api_key = Some(SecretString::from("global"));
    settings.azure_key = Some(SecretString::from("azure"));
    settings.azure_openai_api_key = Some(SecretString::from("env-oai"));
    settings.azure_api_key = Some(SecretString::from("env-old"));
    assert_eq!(exposed(settings.azure_api_key(&request)), Some("request"));
    request.api_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.azure_api_key(&request)), Some("global"));
    settings.api_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.azure_api_key(&request)), Some("azure"));
    settings.azure_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.azure_api_key(&request)), Some("env-oai"));
    settings.azure_openai_api_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.azure_api_key(&request)), Some("env-old"));
    settings.azure_api_key = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.azure_api_key(&request)), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn azure_base_and_version(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    request.api_base = Some("base".to_owned());
    request.api_version = Some("version".to_owned());
    settings.api_base = Some("api_base".to_owned());
    settings.api_version = Some("api_version".to_owned());
    settings.azure_api_base = Some("azure_api_base".to_owned());
    settings.azure_api_version = Some("azure_api_version".to_owned());
    assert_eq!(settings.azure_base(&request), Some("base"));
    assert_eq!(settings.azure_version(&request), Some("version"));
    request.api_base = cleared.map(str::to_owned);
    request.api_version = cleared.map(str::to_owned);
    assert_eq!(settings.azure_base(&request), Some("api_base"));
    assert_eq!(settings.azure_version(&request), Some("api_version"));
    settings.api_base = cleared.map(str::to_owned);
    settings.api_version = cleared.map(str::to_owned);
    assert_eq!(settings.azure_base(&request), Some("azure_api_base"));
    assert_eq!(settings.azure_version(&request), Some("azure_api_version"));
    settings.azure_api_base = cleared.map(str::to_owned);
    settings.azure_api_version = cleared.map(str::to_owned);
    assert_eq!(settings.azure_base(&request), None);
    assert_eq!(settings.azure_version(&request), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn vertex_project_and_location(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    request.vertex_project = Some("project".to_owned());
    request.vertex_location = Some("location".to_owned());
    settings.vertex_project = Some("vertex_project".to_owned());
    settings.vertex_location = Some("vertex_location".to_owned());
    settings.vertexai_project = Some("vertexai_project".to_owned());
    settings.vertexai_location = Some("env-location".to_owned());
    assert_eq!(settings.vertex_project(&request), Some("project"));
    assert_eq!(settings.vertex_location(&request), Some("location"));
    request.vertex_project = cleared.map(str::to_owned);
    request.vertex_location = cleared.map(str::to_owned);
    assert_eq!(settings.vertex_project(&request), Some("vertex_project"));
    assert_eq!(settings.vertex_location(&request), Some("vertex_location"));
    settings.vertex_project = cleared.map(str::to_owned);
    settings.vertex_location = cleared.map(str::to_owned);
    assert_eq!(settings.vertex_project(&request), Some("vertexai_project"));
    assert_eq!(settings.vertex_location(&request), Some("env-location"));
    settings.vertexai_project = cleared.map(str::to_owned);
    settings.vertexai_location = cleared.map(str::to_owned);
    assert_eq!(settings.vertex_project(&request), None);
    assert_eq!(settings.vertex_location(&request), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn vertex_credentials_precedence(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    request.vertex_credentials = Some(SecretString::from("request"));
    settings.vertexai_credentials = Some(SecretString::from("env"));
    assert_eq!(
        exposed(settings.vertex_credentials(&request)),
        Some("request")
    );
    request.vertex_credentials = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.vertex_credentials(&request)), Some("env"));
    settings.vertexai_credentials = cleared.map(SecretString::from);
    assert_eq!(exposed(settings.vertex_credentials(&request)), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn request_only_settings(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    settings.api_version = Some("global-version".to_owned());
    settings.azure_api_version = Some("env-version".to_owned());
    settings.organization = Some("global-org".to_owned());
    settings.openai_organization = Some("env-org".to_owned());
    request.api_version = Some("version".to_owned());
    request.organization = Some("request-org".to_owned());
    assert_eq!(settings.openai_api_version(&request), Some("version"));
    assert_eq!(settings.azure_organization(&request), Some("request-org"));
    request.api_version = cleared.map(str::to_owned);
    request.organization = cleared.map(str::to_owned);
    assert_eq!(settings.openai_api_version(&request), None);
    assert_eq!(settings.azure_organization(&request), None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[test]
fn vertex_resource_settings(#[case] cleared: Option<&str>) {
    let (mut settings, mut request) = test_settings();
    settings.api_base = Some("global-base".to_owned());
    request.api_base = Some("request-base".to_owned());
    assert_eq!(request.api_base.as_deref().unwrap_or(""), "request-base");
    request.api_base = cleared.map(str::to_owned);
    assert_eq!(settings.api_base.as_deref(), Some("global-base"));
    assert_eq!(request.api_base.as_deref().unwrap_or(""), "");
}

#[test]
fn timeout_capability_and_conversion() {
    assert!(supports_httpx_timeout("openai"));
    assert!(supports_httpx_timeout("azure"));
    assert!(supports_httpx_timeout("bedrock"));
    assert!(!supports_httpx_timeout("vertex_ai"));
    assert_eq!(request_timeout(Some(0.0_f64), Some(9.0_f64)), Ok(9.0_f64));
    assert_eq!(request_timeout(Some(0.0_f64), Some(0.0_f64)), Ok(600.0_f64));
    assert_eq!(
        resolve_fine_tuning_timeout(Some(&TimeoutValue::Number(0.0)), "openai", None),
        Ok(FineTuningTimeout::Seconds(600.0))
    );
}

#[rstest]
#[case::nonnumeric("bad")]
#[case::leading_underscore("_1")]
#[case::trailing_underscore("1_")]
#[case::before_decimal("1_.0")]
#[case::after_decimal("1._0")]
#[case::double_underscore("1__0")]
#[test]
fn invalid_timeout_strings(#[case] value: &str) {
    let raw = TimeoutValue::String(value.to_owned());
    assert!(matches!(
        resolve_fine_tuning_timeout(Some(&raw), "openai", None),
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
    let (mut settings, _) = test_settings();
    settings.api_key = Some(SecretString::from("secret"));
    assert!(!format!("{settings:?}").contains("secret"));
}

fn test_settings() -> (super::ResourceSdkSettings, ResourceSdkRequestOptions) {
    let settings = super::ResourceSdkSettings {
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
    };
    (settings, ResourceSdkRequestOptions::default())
}

fn exposed(value: Option<&SecretString>) -> Option<&str> {
    value.map(SecretString::expose_secret)
}

#[rstest]
#[case::absent(None, 600.0_f64)]
#[case::fraction(Some(TimeoutValue::String("7.5".to_owned())), 7.5_f64)]
#[case::whitespace(Some(TimeoutValue::String(" 7 ".to_owned())), 7.0_f64)]
#[case::underscore(Some(TimeoutValue::String("1_000".to_owned())), 1000.0_f64)]
#[case::empty(Some(TimeoutValue::String(String::new())), 600.0_f64)]
#[case::negative(Some(TimeoutValue::Number(-7.0_f64)), -7.0_f64)]
#[case::numeric(Some(TimeoutValue::Number(7.0_f64)), 7.0_f64)]
#[case::text_zero(Some(TimeoutValue::String("0".to_owned())), 0.0_f64)]
#[test]
fn raw_timeout_defaults(#[case] raw: Option<TimeoutValue>, #[case] expected: f64) {
    assert_eq!(
        resolve_fine_tuning_timeout(raw.as_ref(), "openai", None),
        Ok(FineTuningTimeout::Seconds(expected))
    );
}

#[rstest]
#[case::null(None, 600.0_f64)]
#[case::zero(Some(0.0_f64), 600.0_f64)]
#[case::nonzero(Some(7.0_f64), 7.0_f64)]
#[case::negative(Some(-7.0_f64), -7.0_f64)]
#[test]
fn unsupported_timeout_read(#[case] read: Option<f64>, #[case] expected: f64) {
    let structured = StructuredTimeout {
        connect: None,
        read,
        write: None,
        pool: None,
    };
    assert_eq!(
        resolve_fine_tuning_timeout(None, "vertex_ai", Some(structured)),
        Ok(FineTuningTimeout::Seconds(expected))
    );
}

#[rstest]
#[case::nan("nan")]
#[case::infinity("inf")]
#[case::negative_infinity("-inf")]
#[test]
fn nonfinite_strings(#[case] text: &str) {
    let raw = TimeoutValue::String(text.to_owned());
    assert_eq!(
        resolve_fine_tuning_timeout(Some(&raw), "openai", None),
        Err(TimeoutError::NonFinite)
    );
}
