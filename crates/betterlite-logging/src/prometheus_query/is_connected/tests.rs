#![cfg(test)]

use crate::prometheus_query::PrometheusSettings;
use crate::prometheus_query::error::QueryError;
use crate::prometheus_query::fake_transport::FakeFailure;
use rstest::rstest;

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
