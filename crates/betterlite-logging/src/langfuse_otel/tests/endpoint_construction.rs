#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use super::{BASIC_AUTH, PUBLIC_KEY, SECRET_KEY, environment, langfuse_headers};
use crate::langfuse_otel::destination::build_langfuse_otel_config;
use crate::otel::legacy_policy::config::Exporter;

#[rstest]
#[case::no_host_is_us_cloud(None, "https://us.cloud.langfuse.com/api/public/otel")]
#[case::empty_host_is_us_cloud(Some(""), "https://us.cloud.langfuse.com/api/public/otel")]
#[case::bare_host_gets_https(
    Some("langfuse.example.com"),
    "https://langfuse.example.com/api/public/otel"
)]
#[case::bare_host_with_port(Some("localhost:3000"), "https://localhost:3000/api/public/otel")]
#[case::http_is_kept(Some("http://localhost:3000"), "http://localhost:3000/api/public/otel")]
#[case::https_is_kept(
    Some("https://eu.example.com"),
    "https://eu.example.com/api/public/otel"
)]
#[case::trailing_slash_is_removed(
    Some("https://example.com/"),
    "https://example.com/api/public/otel"
)]
#[case::every_trailing_slash_is_removed(
    Some("https://example.com///"),
    "https://example.com/api/public/otel"
)]
#[case::prefix_is_only_checked_for_http(Some("httpbin.org"), "httpbin.org/api/public/otel")]
#[case::path_is_kept(
    Some("https://example.com/langfuse"),
    "https://example.com/langfuse/api/public/otel"
)]
fn endpoint_construction_normalizes_the_host(#[case] host: Option<&str>, #[case] expected: &str) {
    let config = build_langfuse_otel_config(
        PUBLIC_KEY,
        &SecretString::from(SECRET_KEY),
        host,
        &environment(&[]),
    );
    assert_eq!(config.endpoint.as_deref(), Some(expected));
}

#[test]
fn endpoint_construction_sends_otlp_http_with_the_langfuse_headers() {
    let config = build_langfuse_otel_config(
        PUBLIC_KEY,
        &SecretString::from(SECRET_KEY),
        None,
        &environment(&[]),
    );
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(config.headers, Some(langfuse_headers(BASIC_AUTH)));
    assert!(!config.signals.enable_metrics);
    assert!(!config.signals.enable_events);
}

#[rstest]
#[case::defaults(&[], "litellm", "production")]
#[case::from_the_environment(
    &[("OTEL_SERVICE_NAME", "svc"), ("OTEL_ENVIRONMENT_NAME", "staging")],
    "svc",
    "staging"
)]
fn endpoint_construction_applies_the_config_post_init_defaults(
    #[case] variables: &[(&str, &str)],
    #[case] service_name: &str,
    #[case] deployment_environment: &str,
) {
    let config = build_langfuse_otel_config(
        PUBLIC_KEY,
        &SecretString::from(SECRET_KEY),
        None,
        &environment(variables),
    );
    assert_eq!(config.service_name, service_name);
    assert_eq!(config.model_id, service_name);
    assert_eq!(config.deployment_environment, deployment_environment);
}
