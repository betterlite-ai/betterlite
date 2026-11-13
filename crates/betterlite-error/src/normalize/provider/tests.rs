#![cfg(test)]

use super::provider_facts;
use crate::normalize::Normalizer;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

#[rstest]
#[case::vendor_limit(429, include_bytes!("fixtures/vendor_limit.json"), "429_RATE_LIMIT_EXCEEDED")]
#[case::budget_message(429, include_bytes!("fixtures/budget_message.json"), "429_BUDGET_EXCEEDED")]
#[case::context(400, include_bytes!("fixtures/context.json"), "400_CONTEXT_WINDOW_EXCEEDED")]
#[case::invalid_bytes(503, &[255], "503_PROVIDER_OVERLOADED")]
#[case::private_details(500, include_bytes!("fixtures/private_details.json"), "500_PROVIDER_INTERNAL_ERROR")]
fn error_normalization_tests_behavior(
    #[case] status: u16,
    #[case] body: &[u8],
    #[case] expected: &str,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let facts = provider_facts(status, body, &redactor);
    assert_eq!(
        Normalizer::new().unwrap().normalize(Some(&facts)),
        Some(expected)
    );
    assert!(!facts.message.contains("sk-secretsecretsecretsecret"));
    assert!(!facts.message.contains("10.0.0.1"));
    assert!(!facts.message.contains("/home/private"));
}
