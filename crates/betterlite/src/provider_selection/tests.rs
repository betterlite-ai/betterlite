#![cfg(test)]

use super::PROVIDERS;

#[test]
fn catalog_has_the_six_preserved_providers_in_order() {
    let ids: Vec<&str> = PROVIDERS.iter().map(|provider| provider.id).collect();
    assert_eq!(
        ids,
        vec![
            "openai",
            "anthropic",
            "gemini",
            "azure",
            "bedrock",
            "ollama"
        ]
    );
}

#[test]
fn anthropic_carries_eleven_models_and_its_test_model() {
    let anthropic = &PROVIDERS[1];
    assert_eq!(anthropic.models.len(), 11);
    assert_eq!(anthropic.test_model, Some("claude-haiku-4-5-20251001"));
}

#[test]
fn azure_requires_an_api_base_and_carries_an_api_version() {
    let azure = &PROVIDERS[3];
    assert!(azure.needs_api_base);
    assert_eq!(azure.api_version, Some("2024-07-01-preview"));
}

#[test]
fn bedrock_carries_its_extra_keys_and_hints() {
    let bedrock = &PROVIDERS[4];
    assert_eq!(
        bedrock.extra_keys,
        ["AWS_SECRET_ACCESS_KEY", "AWS_REGION_NAME"]
    );
    assert_eq!(bedrock.extra_hints.len(), bedrock.extra_keys.len());
}

#[test]
fn ollama_has_no_env_key_and_a_local_api_base() {
    let ollama = &PROVIDERS[5];
    assert_eq!(ollama.env_key, None);
    assert_eq!(ollama.api_base, Some("http://localhost:11434"));
}
