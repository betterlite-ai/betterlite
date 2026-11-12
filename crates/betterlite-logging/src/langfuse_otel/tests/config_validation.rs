#![cfg(test)]

use rstest::rstest;

use super::{BASIC_AUTH, PUBLIC_KEY, SECRET_KEY, environment, langfuse_headers};
use crate::langfuse_otel::destination::get_langfuse_otel_config;
use crate::langfuse_otel::error::LangfuseOtelError;

#[rstest]
#[case::no_keys(&[])]
#[case::only_public_key(&[("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY)])]
#[case::only_secret_key(&[("LANGFUSE_SECRET_KEY", SECRET_KEY)])]
#[case::empty_public_key(&[("LANGFUSE_PUBLIC_KEY", ""), ("LANGFUSE_SECRET_KEY", SECRET_KEY)])]
#[case::empty_secret_key(&[("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY), ("LANGFUSE_SECRET_KEY", "")])]
fn config_validation_rejects_a_missing_key(#[case] variables: &[(&str, &str)]) {
    let error = get_langfuse_otel_config(&environment(variables)).expect_err("missing key");
    assert!(matches!(error, LangfuseOtelError::MissingCredentials));
    assert_eq!(
        error.to_string(),
        "LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY must be set for Langfuse OpenTelemetry integration."
    );
}

#[rstest]
#[case::us_cloud_default(&[], "https://us.cloud.langfuse.com/api/public/otel")]
#[case::custom_host(&[("LANGFUSE_HOST", "lf.example.com")], "https://lf.example.com/api/public/otel")]
fn config_validation_accepts_both_keys(
    #[case] extra: &[(&str, &str)],
    #[case] expected_endpoint: &str,
) {
    let mut variables = vec![
        ("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY),
        ("LANGFUSE_SECRET_KEY", SECRET_KEY),
    ];
    variables.extend_from_slice(extra);
    let config = get_langfuse_otel_config(&environment(&variables)).expect("config");
    assert_eq!(config.endpoint.as_deref(), Some(expected_endpoint));
    assert_eq!(config.headers, Some(langfuse_headers(BASIC_AUTH)));
}
