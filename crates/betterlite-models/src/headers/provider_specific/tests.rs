#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use crate::headers::provider_specific::{ProviderSpecificHeader, provider_specific_headers};

fn entry(provider: &str, headers: &[(&str, &str)]) -> ProviderSpecificHeader {
    let mut extra_headers = IndexMap::new();
    for (name, value) in headers {
        extra_headers.insert((*name).to_owned(), (*value).to_owned());
    }
    ProviderSpecificHeader {
        custom_llm_provider: provider.to_owned(),
        extra_headers,
    }
}

#[rstest]
#[case::no_entries(&[], Some("openai"), &[])]
#[case::no_provider(&[entry("openai", &[("a", "1")])], None, &[])]
#[case::matching_provider(&[entry("openai", &[("a", "1")])], Some("openai"), &[("a", "1")])]
#[case::another_provider(&[entry("openai", &[("a", "1")])], Some("azure"), &[])]
#[case::a_comma_separated_scope_matches_any_member(
    &[entry("openai, azure", &[("a", "1")])],
    Some("azure"),
    &[("a", "1")]
)]
#[case::scope_matching_is_case_sensitive(&[entry("Openai", &[("a", "1")])], Some("openai"), &[])]
#[case::an_empty_scope_matches_an_empty_provider(&[entry("", &[("a", "1")])], Some(""), &[("a", "1")])]
#[case::entries_merge_and_a_later_entry_wins(
    &[entry("openai", &[("a", "1"), ("b", "1")]), entry("openai", &[("a", "2"), ("c", "3")])],
    Some("openai"),
    &[("a", "2"), ("b", "1"), ("c", "3")]
)]
#[case::an_entry_without_a_scope_contributes_nothing(&[entry("", &[("a", "1")])], Some("openai"), &[])]
fn provider_controls_tests_behavior(
    #[case] entries: &[ProviderSpecificHeader],
    #[case] provider: Option<&str>,
    #[case] expected: &[(&str, &str)],
) {
    let error = serde_json::from_str::<ProviderSpecificHeader>(
        r#"{"custom_llm_provider":"openai","extra_header":{"a":"1"}}"#,
    )
    .expect_err("an unknown entry key is rejected");
    assert!(error.to_string().contains("extra_header"));
    let scoped = provider_specific_headers(Some(entries), provider);
    let rendered: Vec<(&str, &str)> = scoped
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(rendered, expected);
}
