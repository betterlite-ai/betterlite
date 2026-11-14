use std::error::Error as _;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use super::support::{NoSecrets, build, guardrail, litellm_params, priced, secrets, text};
use crate::azure_content_safety::api_version::DEFAULT_API_VERSION;
use crate::azure_content_safety::base::DEFAULT_TIMEOUT;
use crate::azure_prompt_shield::cost::CostTier;
use crate::azure_prompt_shield::error::PromptShieldError;
use crate::azure_prompt_shield::guardrail::AzurePromptShieldGuardrail;

const BASE: &str = "http://127.0.0.1:1";

fn priced_guardrail() -> AzurePromptShieldGuardrail {
    build(
        priced(
            BASE,
            Some(text("paid")),
            Some(GuardrailValue::Decimal(0.25)),
        ),
        &NoSecrets,
    )
}

#[test]
fn an_update_replaces_credentials_pricing_version_and_timeout() {
    let lookup = secrets(&[("AZURE_KEY", "updated-key")]);
    let mut guardrail = guardrail(BASE);
    guardrail
        .update_litellm_params(
            &litellm_params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call",
                    "api_key": "os.environ/AZURE_KEY", "api_base": "http://new-base",
                    "api_version": "2023-01-01", "timeout": 5.0,
                    "cost_tier": "paid", "price_per_1000_text_records": 0.5,
                    "default_on": true}"#,
            ),
            &lookup,
        )
        .unwrap();
    assert_eq!(guardrail.transport.api_key.expose_secret(), "updated-key");
    assert_eq!(guardrail.transport.api_base, "http://new-base");
    assert_eq!(guardrail.transport.api_version, "2023-01-01");
    assert_eq!(guardrail.transport.timeout, Duration::from_secs(5));
    assert_eq!(guardrail.cost_tier, Some(CostTier::Paid));
    assert_eq!(
        guardrail.price_per_1000_text_records,
        Some(Decimal::try_from(0.5).unwrap())
    );
    assert!(guardrail.base.settings.default_on);
}

#[rstest]
#[case::paid_without_price(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "cost_tier": "paid", "api_base": "http://changed"}"#,
    "requires a positive price"
)]
#[case::unknown_tier(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "cost_tier": "premium", "api_base": "http://changed"}"#,
    "must be 'free' or 'paid'"
)]
#[case::bad_price(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "cost_tier": "paid", "price_per_1000_text_records": "cheap", "api_base": "http://changed"}"#,
    "must be a number"
)]
#[case::bad_credential(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "os.environ/MISSING_KEY", "api_base": "http://changed"}"#,
    "unset or blank"
)]
#[case::bad_version(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_version": 2, "api_base": "http://changed"}"#,
    "api_version must be a string"
)]
#[case::bad_timeout(
    r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "timeout": -1.0, "api_base": "http://changed"}"#,
    "timeout must be a non-negative number"
)]
fn an_invalid_update_leaves_the_guardrail_untouched(#[case] update: &str, #[case] message: &str) {
    let mut guardrail = priced_guardrail();
    let error = guardrail
        .update_litellm_params(&litellm_params(update), &NoSecrets)
        .unwrap_err();
    assert!(error.to_string().contains(message), "{error}");
    assert_eq!(guardrail.transport.api_base, BASE);
    assert_eq!(guardrail.transport.api_key.expose_secret(), "azure-key");
    assert_eq!(guardrail.transport.api_version, DEFAULT_API_VERSION);
    assert_eq!(guardrail.transport.timeout, DEFAULT_TIMEOUT);
    assert_eq!(guardrail.cost_tier, Some(CostTier::Paid));
    assert_eq!(
        guardrail.price_per_1000_text_records,
        Some(Decimal::try_from(0.25).unwrap())
    );
}

#[test]
fn an_invalid_timeout_keeps_the_conversion_error_as_its_source() {
    let mut guardrail = priced_guardrail();
    let error = guardrail
        .update_litellm_params(
            &litellm_params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "timeout": -1.0}"#,
            ),
            &NoSecrets,
        )
        .unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::InvalidTimeout { .. }),
        "{error}"
    );
    assert!(error.source().is_some(), "{error}");
}

#[test]
fn an_update_without_credentials_keeps_the_running_ones() {
    let mut guardrail = guardrail(BASE);
    guardrail
        .update_litellm_params(
            &litellm_params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "cost_tier": "free"}"#,
            ),
            &NoSecrets,
        )
        .unwrap();
    assert_eq!(guardrail.transport.api_key.expose_secret(), "azure-key");
    assert_eq!(guardrail.transport.api_base, BASE);
    assert_eq!(guardrail.cost_tier, Some(CostTier::Free));
}

#[test]
fn a_plain_credential_in_an_update_replaces_the_running_key() {
    let mut guardrail = guardrail(BASE);
    guardrail
        .update_litellm_params(
            &litellm_params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "plain-new-key"}"#,
            ),
            &NoSecrets,
        )
        .unwrap();
    assert_eq!(guardrail.transport.api_key.expose_secret(), "plain-new-key");
}

#[test]
fn an_empty_api_version_in_an_update_counts_as_unset() {
    let mut guardrail = guardrail(BASE);
    guardrail
        .update_litellm_params(
            &litellm_params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_version": ""}"#,
            ),
            &NoSecrets,
        )
        .unwrap();
    assert_eq!(guardrail.transport.api_version, DEFAULT_API_VERSION);
}

#[test]
fn a_null_api_version_in_an_update_resets_to_the_default() {
    let mut guardrail = guardrail(BASE);
    guardrail.transport.api_version = "2023-01-01".to_owned();
    guardrail
        .update_litellm_params(
            &litellm_params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_version": null}"#,
            ),
            &NoSecrets,
        )
        .unwrap();
    assert_eq!(guardrail.transport.api_version, DEFAULT_API_VERSION);
}

#[test]
fn an_update_without_a_timeout_restores_the_default() {
    let mut guardrail = guardrail(BASE);
    guardrail.transport.timeout = Duration::from_secs(5);
    guardrail
        .update_litellm_params(
            &litellm_params(r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call"}"#),
            &NoSecrets,
        )
        .unwrap();
    assert_eq!(guardrail.transport.timeout, DEFAULT_TIMEOUT);
}

#[test]
fn an_os_environ_credential_never_overwrites_the_running_key() {
    let mut guardrail = guardrail(BASE);
    let error = guardrail
        .update_litellm_params(
            &litellm_params(
                r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "os.environ/UNSET_KEY"}"#,
            ),
            &NoSecrets,
        )
        .unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::UnsetSecret { .. }),
        "{error}"
    );
    assert_eq!(guardrail.transport.api_key.expose_secret(), "azure-key");
}
