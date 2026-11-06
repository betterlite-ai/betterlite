use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::http_analytics_loggers::athina::{ADDITIONAL_KEYS, AthinaSettings};
use crate::http_analytics_loggers::tests::support::environment;

#[rstest::rstest]
#[case::default_base_url(
    &[("ATHINA_API_KEY", "athina-key-1")],
    "https://log.athina.ai/api/v1/log/inference"
)]
#[case::configured_base_url(
    &[
        ("ATHINA_API_KEY", "athina-key-2"),
        ("ATHINA_BASE_URL", "https://athina.example.com")
    ],
    "https://athina.example.com/api/v1/log/inference"
)]
fn athina_reads_the_key_and_base_url_at_init(
    #[case] pairs: &[(&str, &str)],
    #[case] endpoint: &str,
) {
    let settings = AthinaSettings::from_environment(&environment(pairs));
    assert_eq!(settings.endpoint, endpoint);
    assert_eq!(
        settings.api_key.expect("the key is set").expose_secret(),
        pairs[0].1
    );
}

#[test]
fn athina_leaves_the_key_unset_at_init_without_the_variable() {
    let settings = AthinaSettings::from_environment(&BTreeMap::new());
    assert!(settings.api_key.is_none());
    assert_eq!(
        settings.endpoint,
        "https://log.athina.ai/api/v1/log/inference"
    );
}

#[test]
fn athina_carries_the_thirteen_additional_metadata_keys() {
    assert_eq!(ADDITIONAL_KEYS.len(), 13);
    assert_eq!(ADDITIONAL_KEYS.first(), Some(&"environment"));
    assert_eq!(ADDITIONAL_KEYS.last(), Some(&"custom_attributes"));
}
