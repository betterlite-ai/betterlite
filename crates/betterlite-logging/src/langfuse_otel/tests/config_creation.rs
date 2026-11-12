#![cfg(test)]

use rstest::rstest;

use super::{BASIC_AUTH, PUBLIC_KEY, SECRET_KEY, environment, langfuse_headers};
use crate::langfuse_otel::destination::config_from_langfuse_env;
use crate::otel::legacy_policy::config::Exporter;

#[rstest]
#[case::us_cloud_default(&[], "https://us.cloud.langfuse.com/api/public/otel")]
#[case::langfuse_host(&[("LANGFUSE_HOST", "lf.example.com")], "https://lf.example.com/api/public/otel")]
#[case::otel_host_wins(
    &[("LANGFUSE_HOST", "lf.example.com"), ("LANGFUSE_OTEL_HOST", "http://otel.example.com/")],
    "http://otel.example.com/api/public/otel"
)]
fn config_creation_with_both_keys_builds_the_langfuse_destination(
    #[case] extra: &[(&str, &str)],
    #[case] expected_endpoint: &str,
) {
    let mut variables = vec![
        ("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY),
        ("LANGFUSE_SECRET_KEY", SECRET_KEY),
    ];
    variables.extend_from_slice(extra);
    let config = config_from_langfuse_env(&environment(&variables));
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(config.endpoint.as_deref(), Some(expected_endpoint));
    assert_eq!(config.headers, Some(langfuse_headers(BASIC_AUTH)));
}

#[rstest]
#[case::no_keys(&[])]
#[case::only_public_key(&[("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY)])]
#[case::only_secret_key(&[("LANGFUSE_SECRET_KEY", SECRET_KEY)])]
#[case::empty_public_key(&[("LANGFUSE_PUBLIC_KEY", ""), ("LANGFUSE_SECRET_KEY", SECRET_KEY)])]
#[case::empty_secret_key(&[("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY), ("LANGFUSE_SECRET_KEY", "")])]
fn config_creation_without_both_keys_falls_back_to_the_otel_environment(
    #[case] variables: &[(&str, &str)],
) {
    let mut all = variables.to_vec();
    all.extend_from_slice(&[
        ("OTEL_EXPORTER", "console"),
        ("OTEL_SERVICE_NAME", "fallback-service"),
        ("LANGFUSE_HOST", "lf.example.com"),
    ]);
    let config = config_from_langfuse_env(&environment(&all));
    assert_eq!(config.exporter, Exporter::Console);
    assert_eq!(config.endpoint, None);
    assert_eq!(config.headers, None);
    assert_eq!(config.service_name, "fallback-service");
}

#[test]
fn config_creation_fallback_reads_the_otlp_endpoint() {
    let config = config_from_langfuse_env(&environment(&[
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "https://collector.example/v1/traces",
        ),
        ("OTEL_EXPORTER_OTLP_HEADERS", "x-token=abc"),
    ]));
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(
        config.endpoint.as_deref(),
        Some("https://collector.example/v1/traces")
    );
    assert_eq!(config.headers.as_deref(), Some("x-token=abc"));
}
