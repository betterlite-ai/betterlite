#![cfg(test)]

use super::provider_of;

#[test]
fn provider_of_prefers_the_callers_custom_provider() {
    assert_eq!(
        provider_of("mistral/mistral-ocr-latest", Some("azure")),
        "azure"
    );
}

#[test]
fn provider_of_reads_the_model_prefix_before_the_first_slash() {
    assert_eq!(provider_of("mistral/mistral-ocr-latest", None), "mistral");
    assert_eq!(provider_of("mistral-ocr-latest", None), "");
}
