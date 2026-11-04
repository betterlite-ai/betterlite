#![cfg(test)]

use rstest::rstest;

use super::{DecodeLimits, JSONProviderRegistry, ProviderId, decode_json_provider_registry};

#[test]
fn loads_all_pinned_descriptors_with_defaults() {
    let registry = JSONProviderRegistry::load();
    assert_eq!(registry.providers.len(), 31);
    let helicone = registry
        .get(ProviderId::Helicone)
        .expect("helicone descriptor");
    assert_eq!(helicone.base_url, "https://ai-gateway.helicone.ai/");
    assert_eq!(helicone.base_class, "openai_gpt");
    assert!(helicone.param_mappings.is_empty());
    assert_eq!(helicone.api_key_env, "HELICONE_API_KEY");
    assert!(registry.exists(ProviderId::Helicone));
    assert!(!registry.exists(ProviderId::Cursor));
    let projection = helicone.projection();
    assert_eq!(projection.provider, ProviderId::Helicone);
    assert_eq!(projection.base_url, "https://ai-gateway.helicone.ai/");
    assert_eq!(projection.api_key_env, "HELICONE_API_KEY");
    assert_eq!(projection.api_base_env, None);
    assert!(
        registry
            .get(ProviderId::Sarvam)
            .expect("sarvam")
            .special_handling
            .is_empty()
    );
    assert!(registry.supports_responses_api(ProviderId::Neosantara));
    assert!(!registry.supports_responses_api(ProviderId::ScxAi));
}

#[test]
fn reverse_lookup_is_exact_and_uses_first_insertion() {
    let mut registry = JSONProviderRegistry::default();
    let bytes = br#"{"publicai":{"base_url":"same","api_key_env":"A"},"helicone":{"base_url":"same","api_key_env":"B"}}"#.to_vec();
    registry.load_from(|| Ok(bytes));
    assert_eq!(
        registry.get_by_base_url("same").map(|entry| entry.provider),
        Some(ProviderId::Publicai)
    );
    assert!(registry.get_by_base_url("same/").is_none());
}

#[rstest]
#[case::missing_required(br#"{"publicai":{"api_key_env":"A"}}"#, 0)]
#[case::valid_first_bad_second(br#"{"publicai":{"base_url":"one","api_key_env":"A"},"helicone":{"base_url":1,"api_key_env":"B"}}"#, 1)]
fn failed_load_is_once_only_and_keeps_partial_entries(
    #[case] bytes: &[u8],
    #[case] expected: usize,
) {
    let mut registry = JSONProviderRegistry::default();
    let bytes = bytes.to_vec();
    registry.load_from(|| Ok(bytes));
    assert_eq!(registry.providers.len(), expected);
    let bytes = br#"{"helicone":{"base_url":"two","api_key_env":"B"}}"#.to_vec();
    registry.load_from(|| Ok(bytes));
    assert_eq!(registry.providers.len(), expected);
}

#[test]
fn missing_asset_is_cached_without_retry() {
    let mut registry = JSONProviderRegistry::default();
    let mut reads = 0;
    registry.load_from(|| {
        reads += 1;
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    });
    registry.load_from(|| {
        reads += 1;
        Ok(br#"{"publicai":{"base_url":"later","api_key_env":"KEY"}}"#.to_vec())
    });
    assert_eq!(reads, 1);
    assert_eq!(registry.providers.len(), 0);
}

#[test]
fn decoder_preserves_order_and_enforces_limits() {
    let decoded =
        decode_json_provider_registry(br#"{"helicone":{},"publicai":{}}"#, DecodeLimits::default())
            .expect("valid");
    assert_eq!(
        decoded.keys().map(String::as_str).collect::<Vec<_>>(),
        ["helicone", "publicai"]
    );
    assert!(
        decode_json_provider_registry(
            b"{}",
            DecodeLimits {
                bytes: 1,
                entries: 2,
                depth: 4
            }
        )
        .is_err()
    );
}

#[rstest]
#[case::empty(include_bytes!("../../fuzz/corpus/json_provider_registry/empty.json").as_slice())]
#[case::valid(include_bytes!("../../fuzz/corpus/json_provider_registry/valid.json").as_slice())]
#[case::malformed(include_bytes!("../../fuzz/corpus/json_provider_registry/malformed.json").as_slice())]
fn replays_the_json_provider_registry_corpus(#[case] seed: &[u8]) {
    drop(decode_json_provider_registry(seed, DecodeLimits::default()));
}
