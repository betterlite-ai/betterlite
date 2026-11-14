use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use super::support::{NoSecrets, build, params, priced, secrets, text};
use crate::azure_prompt_shield::cost::CostTier;
use crate::azure_prompt_shield::error::PromptShieldError;
use crate::azure_prompt_shield::guardrail::AzurePromptShieldGuardrail;
use crate::azure_prompt_shield::secret::{ENV_PREFIX, resolved_reference, resolved_secret_value};

#[test]
fn an_environ_reference_resolves_to_the_environment_value() {
    let lookup = secrets(&[("AZURE_PROMPT_SHIELD_KEY", "env-key")]);
    assert_eq!(ENV_PREFIX, "os.environ/");
    assert_eq!(
        resolved_reference("os.environ/AZURE_PROMPT_SHIELD_KEY", &lookup).unwrap(),
        "env-key"
    );
}

#[rstest]
#[case::plain("plain-key")]
#[case::empty("")]
#[case::with_spaces("  padded value  ")]
fn a_value_without_the_prefix_passes_through(#[case] value: &str) {
    assert_eq!(resolved_reference(value, &NoSecrets).unwrap(), value);
}

#[rstest]
#[case::missing(&[])]
#[case::blank(&[("AZURE_KEY", "   ")])]
fn an_unset_or_blank_reference_fails(#[case] pairs: &[(&str, &str)]) {
    let lookup = secrets(pairs);
    let error = resolved_reference("os.environ/AZURE_KEY", &lookup).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::UnsetSecret { .. }),
        "{error}"
    );
    assert!(
        error.to_string().contains("os.environ/AZURE_KEY"),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("unset or blank environment variable"),
        "{error}"
    );
}

#[test]
fn an_empty_reference_name_is_unset() {
    let error = resolved_reference("os.environ/", &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::UnsetSecret { .. }),
        "{error}"
    );
}

#[test]
fn a_non_text_value_is_not_resolved() {
    let price = GuardrailValue::Decimal(2.5);
    assert_eq!(resolved_secret_value(&price, &NoSecrets).unwrap(), price);
    assert_eq!(
        resolved_secret_value(&GuardrailValue::Null, &NoSecrets).unwrap(),
        GuardrailValue::Null
    );
}

#[test]
fn startup_resolves_the_key_base_and_pricing_references() {
    let lookup = secrets(&[
        ("AZURE_KEY", "env-key"),
        ("AZURE_BASE", "http://env-base"),
        ("TIER", "paid"),
        ("PRICE", "0.75"),
    ]);
    let mut params = priced(
        "ignored",
        Some(text("os.environ/TIER")),
        Some(text("os.environ/PRICE")),
    );
    params.api_key = SecretString::from("os.environ/AZURE_KEY".to_owned());
    params.api_base = "os.environ/AZURE_BASE".to_owned();
    let guardrail = build(params, &lookup);
    assert_eq!(guardrail.transport.api_key.expose_secret(), "env-key");
    assert_eq!(guardrail.transport.api_base, "http://env-base");
    assert_eq!(guardrail.cost_tier, Some(CostTier::Paid));
    assert_eq!(
        guardrail.price_per_1000_text_records,
        Some(Decimal::try_from(0.75).unwrap())
    );
}

#[test]
fn a_missing_reference_at_startup_fails_fast() {
    let mut params = params("http://127.0.0.1:1");
    params.api_key = SecretString::from("os.environ/MISSING_KEY".to_owned());
    let error = AzurePromptShieldGuardrail::new(params, &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::UnsetSecret { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("MISSING_KEY"), "{error}");
}
