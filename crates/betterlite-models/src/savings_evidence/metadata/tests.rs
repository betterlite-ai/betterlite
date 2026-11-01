#![cfg(test)]

use super::Metadata;

#[test]
fn deserializes_every_known_key_and_ignores_the_rest() {
    let metadata: Metadata = serde_json::from_str(
        r#"{
            "litellm_gateway_injected_cache": "deployment-a",
            "compression_savings": {"tokens_saved": 5},
            "guardrail_information": {"guardrail_provider": "headroom"},
            "autorouter_savings_estimate": {"version": 2, "status": "estimated"},
            "autorouter_savings": 0.5,
            "unrelated_key": "ignored"
        }"#,
    )
    .expect("every known key deserializes");
    assert!(metadata.litellm_gateway_injected_cache.is_some());
    assert!(metadata.compression_savings.is_some());
    assert!(metadata.guardrail_information.is_some());
    assert!(metadata.autorouter_savings_estimate.is_some());
    assert!(metadata.autorouter_savings.is_some());
}

#[test]
fn defaults_to_every_field_absent() {
    let metadata: Metadata = serde_json::from_str("{}").expect("an empty object deserializes");
    assert!(metadata.litellm_gateway_injected_cache.is_none());
    assert!(metadata.compression_savings.is_none());
}
