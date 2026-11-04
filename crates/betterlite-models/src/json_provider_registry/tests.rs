#![cfg(test)]

use rstest::rstest;
use std::{
    io, str,
    sync::{Arc, Mutex},
};
use tracing::subscriber::with_default;

use super::{DecodeLimits, JSONProviderRegistry, ProviderId, decode_json_provider_registry};

#[test]
fn loads_all_pinned_descriptors_with_defaults() {
    let registry = JSONProviderRegistry::load();
    assert_eq!(registry.providers.len(), 31);
    let helicone = registry.get("helicone").expect("helicone descriptor");
    assert_eq!(helicone.config.base_url, "https://ai-gateway.helicone.ai/");
    assert_eq!(helicone.config.base_class.0, "openai_gpt");
    assert!(helicone.config.param_mappings.is_empty());
    assert_eq!(helicone.config.api_key_env, "HELICONE_API_KEY");
    assert!(registry.exists("helicone"));
    assert!(!registry.exists("cursor"));
    assert!(
        registry
            .get("sarvam")
            .expect("sarvam")
            .config
            .special_handling
            .is_empty()
    );
    assert!(registry.supports_responses_api("neosantara"));
    assert!(!registry.supports_responses_api("scx-ai"));
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
#[case::valid_first_bad_second(br#"{"publicai":{"base_url":"one","api_key_env":"A"},"helicone":{"base_url":1,"api_key_env":"B"},"sarvam":{"base_url":"three","api_key_env":"C"}}"#, 1)]
fn failed_load_is_once_only_and_keeps_partial_entries(
    #[case] bytes: &[u8],
    #[case] expected: usize,
) {
    let mut registry = JSONProviderRegistry::default();
    let initial = bytes.to_vec();
    registry.load_from(|| Ok(initial));
    assert_eq!(registry.providers.len(), expected);
    let retry = br#"{"helicone":{"base_url":"two","api_key_env":"B"}}"#.to_vec();
    registry.load_from(|| Ok(retry));
    assert_eq!(registry.providers.len(), expected);
}

#[test]
fn missing_asset_is_cached_without_retry() {
    let mut registry = JSONProviderRegistry::default();
    let mut reads = 0_usize;
    registry.load_from(|| {
        reads += 1;
        Err(io::Error::from(io::ErrorKind::NotFound))
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
        decode_json_provider_registry(br#"{"helicone":{"base_url":"a","api_key_env":"A"},"publicai":{"base_url":"b","api_key_env":"B"}}"#, DecodeLimits::default())
            .expect("valid");
    assert_eq!(
        decoded
            .providers
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["helicone", "publicai"]
    );
    assert!(matches!(
        decode_json_provider_registry(
            b"{}",
            DecodeLimits {
                bytes: 1,
                entries: 2,
                depth: 4
            }
        ),
        Err(super::RegistryError::TooLarge { limit: 1 })
    ));
}

#[rstest]
#[case::empty(include_bytes!("../../fuzz/corpus/json_provider_registry/empty.json").as_slice())]
#[case::valid(include_bytes!("../../fuzz/corpus/json_provider_registry/valid.json").as_slice())]
#[case::malformed(include_bytes!("../../fuzz/corpus/json_provider_registry/malformed.json").as_slice())]
fn replays_the_json_provider_registry_corpus(#[case] seed: &[u8]) {
    drop(decode_json_provider_registry(seed, DecodeLimits::default()));
}

#[test]
fn null_optional_fields_take_defaults() {
    let mut registry = JSONProviderRegistry::default();
    registry.load_from(|| Ok(br#"{"publicai":{"base_url":"base","api_key_env":"KEY","base_class":null,"param_mappings":null,"constraints":null,"special_handling":null,"supported_endpoints":null}}"#.to_vec()));
    let descriptor = registry.get("publicai").expect("null defaults");
    assert_eq!(descriptor.config.base_class.0, "openai_gpt");
    assert!(descriptor.config.param_mappings.is_empty());
    assert_eq!(descriptor.config.constraints, super::Constraints::default());
    assert!(descriptor.config.special_handling.is_empty());
    assert!(descriptor.config.supported_endpoints.is_empty());
}

#[rstest]
#[case::read_failure(io::ErrorKind::Other)]
#[case::missing(io::ErrorKind::NotFound)]
fn read_failures_never_retry(#[case] kind: io::ErrorKind) {
    let mut registry = JSONProviderRegistry::default();
    registry.load_from(|| Err(io::Error::from(kind)));
    registry.load_from(|| panic!("must not retry a failed read"));
    assert!(registry.providers.is_empty());
}

#[rstest]
#[case::malformed_json(b"{".as_slice())]
#[case::unknown_slug(br#"{"unknown":{"base_url":"a","api_key_env":"K"}}"#)]
#[case::missing_key(br#"{"publicai":{"base_url":"a"}}"#)]
fn invalid_assets_are_loaded_once(#[case] bytes: &[u8]) {
    let mut registry = JSONProviderRegistry::default();
    registry.load_from(|| Ok(bytes.to_vec()));
    registry.load_from(|| panic!("must not retry an invalid asset"));
    assert!(registry.providers.is_empty());
}

#[rstest]
#[case::exact("/v1/responses", true)]
#[case::trailing_slash("/v1/responses/", false)]
#[case::unversioned("/responses", false)]
#[case::case_sensitive("/v1/Responses", false)]
fn responses_support_is_exact(#[case] endpoint: &str, #[case] expected: bool) {
    let source = format!(
        r#"{{"publicai":{{"base_url":"base","api_key_env":"KEY","supported_endpoints":["/v1/messages","{endpoint}"]}}}}"#
    );
    let registry =
        decode_json_provider_registry(source.as_bytes(), DecodeLimits::default()).unwrap();
    assert_eq!(registry.supports_responses_api("publicai"), expected);
    assert!(!registry.supports_responses_api("helicone"));
    assert!(!registry.supports_responses_api("unknown"));
    assert!(registry.get("unknown").is_none());
    assert!(!registry.exists("unknown"));
    assert!(!registry.exists("helicone"));
}

#[rstest]
#[case::bytes_at_boundary(b"{}".as_slice(), 2, 0, 0, true)]
#[case::bytes_over_boundary(b"{}".as_slice(), 1, 0, 0, false)]
#[case::entries_at_boundary(br#"{"publicai":{}}"#, 100, 1, 2, true)]
#[case::entries_over_boundary(br#"{"publicai":{}}"#, 100, 0, 2, false)]
#[case::depth_at_boundary(br#"{"publicai":{"unknown":[1]}}"#, 100, 1, 3, true)]
#[case::depth_over_boundary(br#"{"publicai":{"unknown":[1]}}"#, 100, 1, 2, false)]
#[case::object_depth(br#"{"publicai":{"unknown":{"inner":{}}}}"#, 100, 1, 3, false)]
fn decoder_limit_boundaries(
    #[case] bytes: &[u8],
    #[case] maximum: usize,
    #[case] entries: usize,
    #[case] depth: usize,
    #[case] valid: bool,
) {
    assert_eq!(
        decode_json_provider_registry(
            bytes,
            DecodeLimits {
                bytes: maximum,
                entries,
                depth
            }
        )
        .is_ok(),
        valid
    );
}

#[test]
fn successful_load_is_once_only_and_keeps_descriptor_values() {
    let mut registry = JSONProviderRegistry::load();
    registry.load_from(|| panic!("must not reload the asset"));
    let publicai = &registry.get("publicai").unwrap().config;
    assert_eq!(publicai.api_base_env.as_deref(), Some("PUBLICAI_API_BASE"));
    assert_eq!(
        publicai
            .param_mappings
            .get("max_completion_tokens")
            .map(String::as_str),
        Some("max_tokens")
    );
    assert_eq!(
        publicai
            .special_handling
            .get("convert_content_list_to_string"),
        Some(&true)
    );
    assert_eq!(
        registry
            .get("scx-ai")
            .unwrap()
            .config
            .constraints
            .temperature_max,
        Some(rust_decimal::Decimal::new(199, 2))
    );
}

#[rstest]
#[case::missing(io::ErrorKind::NotFound, false)]
#[case::read_error(io::ErrorKind::Other, true)]
fn loader_warns_only_for_read_errors(#[case] kind: io::ErrorKind, #[case] warning: bool) {
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    with_default(subscriber, || {
        let mut registry = JSONProviderRegistry::default();
        registry.load_from(|| Err(io::Error::from(kind)));
    });
    let bytes = output.lock().unwrap();
    assert_eq!(str::from_utf8(&bytes).unwrap().contains("WARN"), warning);
}

#[rstest]
#[case::syntax("{")]
#[case::entry(r#"{"publicai":{"api_key_env":"KEY"}}"#)]
fn malformed_assets_warn(#[case] source: &str) {
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    with_default(subscriber, || {
        let mut registry = JSONProviderRegistry::default();
        registry.load_from(|| Ok(source.as_bytes().to_vec()));
    });
    assert!(
        str::from_utf8(&output.lock().unwrap())
            .unwrap()
            .contains("WARN")
    );
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut output) = self.0.lock() else {
            return Err(io::Error::other("poisoned capture"));
        };
        output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn explicit_optional_fields_are_retained() {
    let source = br#"{"publicai":{"base_url":"","api_key_env":"","base_class":"","param_mappings":{"a":"b"},"constraints":{"temperature_max":1.99,"temperature_min":0,"temperature_min_with_n_gt_1":0.3,"cache_control_ttl":true},"special_handling":{"force_store_false":false},"supported_endpoints":["/v1/messages"]}}"#;
    let registry = decode_json_provider_registry(source, DecodeLimits::default()).unwrap();
    let config = &registry.get("publicai").unwrap().config;
    assert!(config.base_url.is_empty());
    assert!(config.api_key_env.is_empty());
    assert!(config.base_class.0.is_empty());
    assert_eq!(
        config.param_mappings.get("a").map(String::as_str),
        Some("b")
    );
    assert_eq!(
        config.constraints.temperature_max,
        Some(rust_decimal::Decimal::new(199, 2))
    );
    assert_eq!(
        config.constraints.temperature_min,
        Some(rust_decimal::Decimal::ZERO)
    );
    assert_eq!(
        config.constraints.temperature_min_with_n_gt_1,
        Some(rust_decimal::Decimal::new(3, 1))
    );
    assert_eq!(config.constraints.cache_control_ttl, Some(true));
    assert_eq!(
        config.special_handling.get("force_store_false"),
        Some(&false)
    );
    assert_eq!(config.supported_endpoints, ["/v1/messages"]);
}
