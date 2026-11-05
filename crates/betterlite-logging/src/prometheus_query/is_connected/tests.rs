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

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::set(Some("http://prometheus:9090"), Some("http://prometheus:9090"))]
#[case::unset(None, None)]
#[case::empty(Some(""), None)]
fn from_lookup_reads_prometheus_url(#[case] value: Option<&str>, #[case] expected: Option<&str>) {
    let pairs: Vec<(&str, &str)> = value
        .map(|url| ("PROMETHEUS_URL", url))
        .into_iter()
        .collect();
    let settings = PrometheusSettings::from_lookup(&environment(&pairs));
    assert_eq!(settings.url.as_deref(), expected);
    assert_eq!(settings.selected_instance, None);
}

#[rstest]
#[case::set(Some("web-1:8000"), Some("web-1:8000"))]
#[case::unset(None, None)]
#[case::empty(Some(""), None)]
fn from_lookup_reads_prometheus_selected_instance(
    #[case] value: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let pairs: Vec<(&str, &str)> = value
        .map(|instance| ("PROMETHEUS_SELECTED_INSTANCE", instance))
        .into_iter()
        .collect();
    let settings = PrometheusSettings::from_lookup(&environment(&pairs));
    assert_eq!(settings.selected_instance.as_deref(), expected);
    assert_eq!(settings.url, None);
}

#[test]
fn from_lookup_feeds_the_endpoint_a_litellm_style_environment() {
    let settings = PrometheusSettings::from_lookup(&environment(&[
        ("PROMETHEUS_URL", "http://prometheus:9090"),
        ("PROMETHEUS_SELECTED_INSTANCE", "web-1:8000"),
    ]));
    assert_eq!(
        settings.endpoint::<FakeFailure>().unwrap(),
        "http://prometheus:9090"
    );
    assert_eq!(settings.selected_instance.as_deref(), Some("web-1:8000"));
}
