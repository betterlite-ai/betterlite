#![cfg(test)]

use crate::prometheus_query::PrometheusSettings;
use crate::prometheus_query::error::QueryError;
use crate::prometheus_query::fake_transport::FakeFailure;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::set(Some("http://prometheus:9090"), true)]
#[case::unset(None, false)]
#[case::empty(Some(""), false)]
fn endpoint_resolves_only_when_the_url_is_set(#[case] url: Option<&str>, #[case] connected: bool) {
    let settings = PrometheusSettings {
        url: url.map(str::to_owned),
        selected_instance: None,
    };
    let endpoint = settings.endpoint::<FakeFailure>();
    assert_eq!(endpoint.is_ok(), connected);
}

#[test]
fn endpoint_returns_the_url_unchanged() {
    let settings = PrometheusSettings {
        url: Some("http://prometheus:9090/".to_owned()),
        selected_instance: Some("host:1".to_owned()),
    };
    assert_eq!(
        settings.endpoint::<FakeFailure>().unwrap(),
        "http://prometheus:9090/"
    );
}

#[test]
fn an_unset_url_fails_with_the_litellm_message() {
    let error = PrometheusSettings::default()
        .endpoint::<FakeFailure>()
        .unwrap_err();
    assert!(matches!(error, QueryError::NotConfigured));
    assert_eq!(
        error.to_string(),
        "PROMETHEUS_URL not set please set 'PROMETHEUS_URL=<>' in .env"
    );
}

fn environment(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .filter_map(|(name, value)| value.map(|text| ((*name).to_owned(), text.to_owned())))
        .collect()
}

#[rstest]
#[case::both_set(
    Some("http://prometheus:9090"),
    Some("web-1:8000"),
    Some("http://prometheus:9090"),
    Some("web-1:8000")
)]
#[case::both_unset(None, None, None, None)]
#[case::both_empty(Some(""), Some(""), None, None)]
#[case::only_url(
    Some("http://prometheus:9090"),
    None,
    Some("http://prometheus:9090"),
    None
)]
#[case::only_instance(None, Some("web-1:8000"), None, Some("web-1:8000"))]
fn from_lookup_reads_both_variables(
    #[case] url: Option<&str>,
    #[case] instance: Option<&str>,
    #[case] expected_url: Option<&str>,
    #[case] expected_instance: Option<&str>,
) {
    let settings = PrometheusSettings::from_lookup(&environment(&[
        ("PROMETHEUS_URL", url),
        ("PROMETHEUS_SELECTED_INSTANCE", instance),
    ]));
    assert_eq!(settings.url.as_deref(), expected_url);
    assert_eq!(settings.selected_instance.as_deref(), expected_instance);
}

#[test]
fn from_lookup_feeds_the_endpoint_a_litellm_style_environment() {
    let settings = PrometheusSettings::from_lookup(&environment(&[
        ("PROMETHEUS_URL", Some("http://prometheus:9090")),
        ("PROMETHEUS_SELECTED_INSTANCE", Some("web-1:8000")),
    ]));
    assert_eq!(
        settings.endpoint::<FakeFailure>().unwrap(),
        "http://prometheus:9090"
    );
    assert_eq!(settings.selected_instance.as_deref(), Some("web-1:8000"));
}
