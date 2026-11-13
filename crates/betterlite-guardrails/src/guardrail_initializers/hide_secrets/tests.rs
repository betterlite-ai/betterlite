#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;

use super::initialize_hide_secrets;
use crate::guardrail_initializers::error::InitializerError;

#[test]
fn hide_secrets_always_reports_the_missing_enterprise_package() {
    let params = decode_litellm_params(
        br#"{"guardrail": "hide-secrets", "mode": "pre_call", "detect_secrets_config": {"detectors": []}}"#,
    )
    .unwrap();
    let error = initialize_hide_secrets(&params, Some("secrets")).unwrap_err();
    assert!(matches!(error, InitializerError::MissingEnterprisePackage));
    assert_eq!(
        error.to_string(),
        "Trying to use Secret DetectionMissing litellm-enterprise package. Please install it to use this feature. Run `pip install litellm-enterprise`"
    );
}

#[test]
fn hide_secrets_fails_even_with_no_configuration() {
    let params =
        decode_litellm_params(br#"{"guardrail": "hide-secrets", "mode": "pre_call"}"#).unwrap();
    assert!(matches!(
        initialize_hide_secrets(&params, None),
        Err(InitializerError::MissingEnterprisePackage)
    ));
}
