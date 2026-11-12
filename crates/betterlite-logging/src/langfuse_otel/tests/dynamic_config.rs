#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{BASIC_AUTH, PUBLIC_KEY, SECRET_KEY, environment, langfuse_headers, params, text};
use crate::langfuse_otel::dynamic::construct_dynamic_otel_config;
use crate::langfuse_otel::error::LangfuseOtelError;
use crate::otel::legacy_policy::config::Exporter;

fn credentials() -> Vec<(&'static str, Json)> {
    vec![
        ("langfuse_public_key", text(PUBLIC_KEY)),
        ("langfuse_secret_key", text(SECRET_KEY)),
    ]
}

#[rstest]
#[case::us_cloud_default(&[], None, "https://us.cloud.langfuse.com/api/public/otel")]
#[case::environment_host(&[("LANGFUSE_HOST", "env.example.com")], None, "https://env.example.com/api/public/otel")]
#[case::otel_host_before_langfuse_host(
    &[("LANGFUSE_HOST", "env.example.com"), ("LANGFUSE_OTEL_HOST", "otel.example.com")],
    None,
    "https://otel.example.com/api/public/otel"
)]
#[case::dynamic_host_wins(
    &[("LANGFUSE_HOST", "env.example.com")],
    Some("team.example.com"),
    "https://team.example.com/api/public/otel"
)]
#[case::empty_dynamic_host_falls_back(
    &[("LANGFUSE_HOST", "env.example.com")],
    Some(""),
    "https://env.example.com/api/public/otel"
)]
fn dynamic_config_builds_a_complete_destination(
    #[case] variables: &[(&str, &str)],
    #[case] dynamic_host: Option<&str>,
    #[case] expected_endpoint: &str,
) {
    let mut fields = credentials();
    if let Some(host) = dynamic_host {
        fields.push(("langfuse_host", text(host)));
    }
    let config = construct_dynamic_otel_config(&params(&fields), &environment(variables))
        .expect("config")
        .expect("a destination");
    assert_eq!(config.exporter, Exporter::Named("otlp_http".to_owned()));
    assert_eq!(config.endpoint.as_deref(), Some(expected_endpoint));
    assert_eq!(config.headers, Some(langfuse_headers(BASIC_AUTH)));
}

#[test]
fn dynamic_config_ignores_the_global_langfuse_keys() {
    let config = construct_dynamic_otel_config(
        &params(&credentials()),
        &environment(&[
            ("LANGFUSE_PUBLIC_KEY", "env-pk"),
            ("LANGFUSE_SECRET_KEY", "env-sk"),
        ]),
    )
    .expect("config")
    .expect("a destination");
    assert_eq!(config.headers, Some(langfuse_headers(BASIC_AUTH)));
}

#[rstest]
#[case::no_params(Json::Null)]
#[case::empty_params(params(&[]))]
#[case::missing_secret(params(&[("langfuse_public_key", text(PUBLIC_KEY)), ("langfuse_host", text("h.example"))]))]
#[case::missing_public(params(&[("langfuse_secret_key", text(SECRET_KEY))]))]
#[case::empty_secret(params(&[("langfuse_public_key", text(PUBLIC_KEY)), ("langfuse_secret_key", text(""))]))]
fn dynamic_config_without_both_keys_is_none(#[case] given: Json) {
    let config = construct_dynamic_otel_config(&given, &environment(&[])).expect("config");
    assert!(config.is_none());
}

#[test]
fn dynamic_config_rejects_a_host_of_the_wrong_type() {
    let mut fields = credentials();
    fields.push(("langfuse_host", Json::Bool(true)));
    let error =
        construct_dynamic_otel_config(&params(&fields), &environment(&[])).expect_err("wrong type");
    assert!(matches!(
        error,
        LangfuseOtelError::DynamicFieldType {
            field: "langfuse_host"
        }
    ));
}

#[test]
fn dynamic_config_rejects_params_that_are_not_an_object() {
    let error =
        construct_dynamic_otel_config(&text("pk"), &environment(&[])).expect_err("not an object");
    assert!(matches!(error, LangfuseOtelError::DynamicParamsType));
}
