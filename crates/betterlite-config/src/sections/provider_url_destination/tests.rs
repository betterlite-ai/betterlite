#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigError;
use crate::sections::Section as _;
use crate::sections::provider_url_destination::Raw;
use crate::sections::provider_url_destination::pattern::{AllowedDestination, Scheme};

fn load(yaml: &str) -> Option<Vec<String>> {
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    let settings = raw.convert(&BTreeMap::<String, String>::new()).ok()?;
    Some(
        settings
            .allowed_hosts
            .iter()
            .map(|entry| format!("{}|{:?}|{:?}", entry.host, entry.scheme, entry.port))
            .collect(),
    )
}

#[rstest]
#[case::defaults("{}", &[])]
#[case::bare_host(
    "litellm_settings: {provider_url_destination_allowed_hosts: [api.example.com]}",
    &["api.example.com|None|None"]
)]
#[case::origin_takes_the_scheme_default_port(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['https://api.example.com']}",
    &["api.example.com|Some(Https)|Some(443)"]
)]
#[case::http_origin_takes_port_80(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['http://api.example.com']}",
    &["api.example.com|Some(Http)|Some(80)"]
)]
#[case::host_and_port(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['api.example.com:8443']}",
    &["api.example.com|None|Some(8443)"]
)]
#[case::bare_string_is_one_entry(
    "litellm_settings: {provider_url_destination_allowed_hosts: api.example.com}",
    &["api.example.com|None|None"]
)]
#[case::litellm_null_is_empty(
    "litellm_settings: {provider_url_destination_allowed_hosts: null}",
    &[]
)]
#[case::case_and_trailing_dots_are_normalized(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['API.Example.COM.']}",
    &["api.example.com|None|None"]
)]
#[case::ipv6_is_bracketed_in_the_entry(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['[::1]:8080']}",
    &["::1|None|Some(8080)"]
)]
#[case::an_ipvfuture_host_is_kept(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['[v1.a]:80']}",
    &["v1.a|None|Some(80)"]
)]
#[case::an_ipvfuture_host_needs_a_version(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['[v.a]']}",
    &[]
)]
#[case::an_ipvfuture_host_needs_a_label(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['[v1.]']}",
    &[]
)]
#[case::invalid_entries_are_skipped(
    "litellm_settings: {provider_url_destination_allowed_hosts: ['ftp://api.example.com', 'user@api.example.com', 'api.example.com:8o', '[zz]', 'http://[::1', '']}",
    &[]
)]
#[case::general_replaces(
    "litellm_settings: {provider_url_destination_allowed_hosts: [a.test]}\ngeneral_settings: {provider_url_destination_allowed_hosts: [b.test]}",
    &["b.test|None|None"]
)]
#[case::general_empty_replaces(
    "litellm_settings: {provider_url_destination_allowed_hosts: [a.test]}\ngeneral_settings: {provider_url_destination_allowed_hosts: []}",
    &[]
)]
#[case::general_null_replaces_with_empty(
    "litellm_settings: {provider_url_destination_allowed_hosts: [a.test]}\ngeneral_settings: {provider_url_destination_allowed_hosts: null}",
    &[]
)]
#[case::general_absent_keeps(
    "litellm_settings: {provider_url_destination_allowed_hosts: [a.test]}\ngeneral_settings: {user_url_validation: true}",
    &["a.test|None|None"]
)]
fn http_boundaries_url_boundary_tests_behavior(#[case] yaml: &str, #[case] expected: &[&str]) {
    let wanted: Vec<String> = expected.iter().map(|entry| (*entry).to_owned()).collect();
    assert_eq!(load(yaml).unwrap(), wanted);
}

#[rstest]
#[case::at_the_limit(2048, true)]
#[case::one_past_the_limit(2049, false)]
fn applies_the_entry_length_bound_exactly(#[case] length: usize, #[case] accepted: bool) {
    let parsed = AllowedDestination::parse(&"a".repeat(length));
    assert_eq!(parsed.is_some(), accepted);
}

#[test]
fn environment_references_resolve_like_the_rest_of_the_config() {
    let yaml = "litellm_settings: {provider_url_destination_allowed_hosts: [os.environ/HOST]}\ngeneral_settings: {provider_url_destination_allowed_hosts: [os.environ/GENERAL]}";
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    let only_host = BTreeMap::from([("HOST".to_owned(), "a.test".to_owned())]);
    assert!(matches!(
        raw.convert(&only_host),
        Err(ConfigError::MissingEnvironment { .. })
    ));
    let with_general: Raw = serde_saphyr::from_str(yaml).unwrap();
    let both = BTreeMap::from([
        ("HOST".to_owned(), "a.test".to_owned()),
        ("GENERAL".to_owned(), "https://b.test".to_owned()),
    ]);
    let settings = with_general.convert(&both).unwrap();
    assert_eq!(
        settings.allowed_hosts,
        vec![AllowedDestination {
            host: "b.test".to_owned(),
            scheme: Some(Scheme::Https),
            port: Some(443),
        }]
    );
}
