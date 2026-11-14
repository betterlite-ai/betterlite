use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{NoSecrets, build, priced, secrets, text};
use crate::azure_prompt_shield::cost::{CostTier, resolve_cost_tier, resolve_price};
use crate::azure_prompt_shield::error::PromptShieldError;
use crate::azure_prompt_shield::guardrail::AzurePromptShieldGuardrail;

#[rstest]
#[case::free("free", Some(CostTier::Free))]
#[case::paid("paid", Some(CostTier::Paid))]
#[case::mixed_case("PaId", Some(CostTier::Paid))]
#[case::padded("  paid ", Some(CostTier::Paid))]
#[case::blank("", None)]
#[case::spaces("   ", None)]
fn a_configured_tier_normalizes_to_free_paid_or_none(
    #[case] configured: &str,
    #[case] expected: Option<CostTier>,
) {
    let raw = text(configured);
    assert_eq!(resolve_cost_tier(Some(&raw), &NoSecrets).unwrap(), expected);
}

#[test]
fn an_unset_or_null_tier_is_no_tier() {
    assert_eq!(resolve_cost_tier(None, &NoSecrets).unwrap(), None);
    let null = GuardrailValue::Null;
    assert_eq!(resolve_cost_tier(Some(&null), &NoSecrets).unwrap(), None);
}

#[rstest]
#[case::unknown_word(text("premium"), "\"premium\"")]
#[case::boolean(GuardrailValue::Boolean(true), "True")]
#[case::number(GuardrailValue::SignedInteger(2), "2")]
fn a_value_that_is_not_a_tier_is_rejected(
    #[case] configured: GuardrailValue,
    #[case] rendered: &str,
) {
    let error = resolve_cost_tier(Some(&configured), &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::InvalidCostTier { .. }),
        "{error}"
    );
    assert!(
        error.to_string().contains("must be 'free' or 'paid'"),
        "{error}"
    );
    assert!(error.to_string().contains(rendered), "{error}");
}

#[test]
fn a_tier_read_from_the_environment_is_normalized_too() {
    let lookup = secrets(&[("PROMPT_SHIELD_TIER", " Paid ")]);
    let reference = text("os.environ/PROMPT_SHIELD_TIER");
    assert_eq!(
        resolve_cost_tier(Some(&reference), &lookup).unwrap(),
        Some(CostTier::Paid)
    );
}

#[rstest]
#[case::missing(None)]
#[case::null(Some(GuardrailValue::Null))]
#[case::zero(Some(GuardrailValue::UnsignedInteger(0)))]
#[case::negative(Some(GuardrailValue::SignedInteger(-1)))]
#[case::zero_text(Some(text("0")))]
fn a_paid_tier_requires_a_positive_price(#[case] price: Option<GuardrailValue>) {
    let error = resolve_price(price.as_ref(), Some(CostTier::Paid), &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::PaidTierWithoutPrice),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("requires a positive price_per_1000_text_records"),
        "{error}"
    );
}

#[test]
fn a_paid_tier_accepts_a_positive_price() {
    let price = text("0.5");
    assert_eq!(
        resolve_price(Some(&price), Some(CostTier::Paid), &NoSecrets).unwrap(),
        Some(Decimal::try_from(0.5).unwrap())
    );
}

#[test]
fn a_free_tier_keeps_the_configured_price() {
    let price = GuardrailValue::Decimal(5.0);
    assert_eq!(
        resolve_price(Some(&price), Some(CostTier::Free), &NoSecrets).unwrap(),
        Some(Decimal::new(5, 0))
    );
}

#[test]
fn no_tier_with_no_price_tracks_usage_only() {
    assert_eq!(resolve_price(None, None, &NoSecrets).unwrap(), None);
}

#[test]
fn a_boolean_price_is_not_a_number() {
    let price = GuardrailValue::Boolean(true);
    let error = resolve_price(Some(&price), None, &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::PriceNotANumber { .. }),
        "{error}"
    );
}

#[test]
fn a_word_price_is_not_a_number() {
    let price = text("cheap");
    let error = resolve_price(Some(&price), None, &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::PriceNotANumber { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("\"cheap\""), "{error}");
}

#[test]
fn a_negative_price_is_rejected() {
    let price = GuardrailValue::Decimal(-1.0);
    let error = resolve_price(Some(&price), None, &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::InvalidPrice { .. }),
        "{error}"
    );
}

#[test]
fn a_paid_deployment_without_a_price_fails_to_start() {
    let params = priced(
        "http://127.0.0.1:1",
        Some(text("paid")),
        Some(GuardrailValue::Null),
    );
    let error = AzurePromptShieldGuardrail::new(params, &NoSecrets).unwrap_err();
    assert!(
        matches!(&error, PromptShieldError::PaidTierWithoutPrice),
        "{error}"
    );
}

#[test]
fn the_guardrail_stores_the_resolved_tier_and_price() {
    let params = priced(
        "http://127.0.0.1:1",
        Some(text("paid")),
        Some(GuardrailValue::Decimal(0.25)),
    );
    let guardrail = build(params, &NoSecrets);
    assert_eq!(guardrail.cost_tier, Some(CostTier::Paid));
    assert_eq!(
        guardrail.price_per_1000_text_records,
        Some(Decimal::try_from(0.25).unwrap())
    );
}
