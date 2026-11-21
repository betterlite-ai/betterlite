#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::SearchOptions;
use super::params::{merged_params, provider_params};
use super::support::{json, object};

fn every_option() -> SearchOptions {
    SearchOptions {
        max_results: Some(5_i64),
        search_domain_filter: Some(vec!["arxiv.org".to_owned(), "nature.com".to_owned()]),
        max_tokens_per_page: Some(1024_i64),
        country: Some("US".to_owned()),
    }
}

#[test]
fn optional_params_hold_every_explicit_option() {
    let params = merged_params(&every_option(), &IndexMap::new());
    let expected: IndexMap<String, Json> = serde_json::from_str(
        r#"{"max_results":5,"search_domain_filter":["arxiv.org","nature.com"],"max_tokens_per_page":1024,"country":"US"}"#,
    )
    .unwrap();
    assert_eq!(params, expected);
    assert_eq!(
        params.keys().collect::<Vec<_>>(),
        [
            "max_results",
            "search_domain_filter",
            "max_tokens_per_page",
            "country"
        ]
    );
}

#[test]
fn optional_params_are_empty_without_options() {
    assert!(merged_params(&SearchOptions::default(), &IndexMap::new()).is_empty());
}

#[rstest]
#[case::max_results(SearchOptions { max_results: Some(0_i64), search_domain_filter: None, max_tokens_per_page: None, country: None }, r#"{"max_results":0}"#)]
#[case::domains(SearchOptions { max_results: None, search_domain_filter: Some(Vec::new()), max_tokens_per_page: None, country: None }, r#"{"search_domain_filter":[]}"#)]
#[case::max_tokens(SearchOptions { max_results: None, search_domain_filter: None, max_tokens_per_page: Some(0_i64), country: None }, r#"{"max_tokens_per_page":0}"#)]
#[case::country(SearchOptions { max_results: None, search_domain_filter: None, max_tokens_per_page: None, country: Some(String::new()) }, r#"{"country":""}"#)]
fn optional_params_keep_explicit_zero_and_empty_values(
    #[case] options: SearchOptions,
    #[case] expected: &str,
) {
    let params = merged_params(&options, &IndexMap::new());
    assert_eq!(Json::Object(params), json(expected));
}

#[test]
fn optional_params_reach_the_provider_as_the_same_object() {
    let params = merged_params(&every_option(), &IndexMap::new());
    assert_eq!(
        provider_params(&params).unwrap(),
        object(
            r#"{"max_results":5,"search_domain_filter":["arxiv.org","nature.com"],"max_tokens_per_page":1024,"country":"US"}"#
        )
    );
}
