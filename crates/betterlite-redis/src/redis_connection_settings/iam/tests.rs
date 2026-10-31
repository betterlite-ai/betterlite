#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use super::apply_iam_precedence;
use crate::redis_connection_settings::model::scalars::{BoolOrText, PassthroughValue};
use crate::redis_connection_settings::model::settings::{RedisConnectionSettings, RedisOverrides};

fn blank_overrides() -> RedisOverrides {
    RedisOverrides::default()
}

fn blank_settings() -> RedisConnectionSettings {
    RedisConnectionSettings::default()
}

#[test]
fn gcp_outranks_azure_and_aws_when_all_three_are_configured() {
    let mut overrides = blank_overrides();
    overrides.gcp_service_account = Some("svc@project.iam.gserviceaccount.com".to_owned());
    overrides.azure_redis_ad_token = Some(SecretString::from("true".to_owned()));
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    let mut settings = blank_settings();

    apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect("gcp wins, so aws's tls requirement never fires");

    assert!(!settings.credential_provider);
}

#[test]
fn azure_outranks_aws_when_both_are_configured() {
    let mut overrides = blank_overrides();
    overrides.azure_redis_ad_token = Some(SecretString::from("true".to_owned()));
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    let mut settings = blank_settings();

    apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect("azure wins, so aws's tls requirement never fires");

    assert!(!settings.credential_provider);
}

#[test]
fn aws_iam_without_tls_is_rejected() {
    let mut overrides = blank_overrides();
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    let mut settings = blank_settings();
    settings.ssl = Some(false);

    let error = apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect_err("aws iam over a plaintext connection must be rejected");
    assert!(error.to_string().contains("requires TLS"));
}

#[test]
fn aws_iam_missing_settings_are_named() {
    let mut overrides = blank_overrides();
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    let mut settings = blank_settings();
    settings.ssl = Some(true);

    let error = apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect_err("aws iam without user_name/cache_name/region must be rejected");
    let message = error.to_string();
    assert!(message.contains("aws_iam_user_name"));
    assert!(message.contains("aws_iam_cache_name"));
    assert!(message.contains("aws_iam_region"));
}

#[test]
fn aws_iam_with_tls_and_every_setting_enables_the_credential_provider() {
    let mut overrides = blank_overrides();
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    overrides.aws_iam_user_name = Some("my-user".to_owned());
    overrides.aws_iam_cache_name = Some("my-cache".to_owned());
    overrides.aws_iam_region = Some("us-east-1".to_owned());
    overrides.username = Some("explicit-user".to_owned());
    overrides.password = Some(Some(SecretString::from("explicit-password".to_owned())));
    let mut settings = blank_settings();
    settings.ssl = Some(true);
    settings.username = overrides.username.clone();
    settings.password = overrides.password.clone().flatten();

    apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect("every required aws iam setting is present");

    assert!(settings.credential_provider);
}

#[test]
fn aws_iam_region_falls_back_to_the_aws_sdk_environment_variables() {
    let mut overrides = blank_overrides();
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    overrides.aws_iam_user_name = Some("my-user".to_owned());
    overrides.aws_iam_cache_name = Some("my-cache".to_owned());
    let mut settings = blank_settings();
    settings.ssl = Some(true);

    let mut env = BTreeMap::new();
    env.insert("AWS_DEFAULT_REGION".to_owned(), "eu-west-1".to_owned());

    apply_iam_precedence(&overrides, &mut settings, &env)
        .expect("aws_iam_region falls back to AWS_DEFAULT_REGION");
    assert!(settings.credential_provider);
}

#[test]
fn gcp_adds_ssl_ca_certs_to_passthrough_only_when_ssl_is_enabled() {
    let mut overrides = blank_overrides();
    overrides.gcp_service_account = Some("svc@project.iam.gserviceaccount.com".to_owned());
    overrides.gcp_ssl_ca_certs = Some("/etc/ca.pem".to_owned());
    let mut settings = blank_settings();
    settings.ssl = Some(true);

    apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect("gcp with ssl enabled never errors");

    assert_eq!(
        settings.passthrough.get("ssl_ca_certs"),
        Some(&PassthroughValue::Text("/etc/ca.pem".to_owned()))
    );
    assert!(!settings.credential_provider);
}

#[test]
fn credential_provider_already_set_short_circuits_iam_precedence() {
    let mut overrides = blank_overrides();
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    let mut settings = blank_settings();
    settings.ssl = Some(false);
    settings.credential_provider = true;

    apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect("an explicit credential_provider skips every iam branch, tls included");
    assert!(settings.credential_provider);
}

#[test]
fn explicit_fields_outrank_environment_fallbacks() {
    let mut overrides = blank_overrides();
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    overrides.aws_iam_user_name = Some("explicit-user".to_owned());
    overrides.aws_iam_cache_name = Some("explicit-cache".to_owned());
    overrides.aws_iam_region = Some("explicit-region".to_owned());
    let mut settings = blank_settings();
    settings.ssl = Some(true);

    let mut env = BTreeMap::new();
    env.insert("REDIS_AWS_IAM_REGION".to_owned(), "env-region".to_owned());

    apply_iam_precedence(&overrides, &mut settings, &env)
        .expect("every required aws iam setting is present");
    assert!(settings.credential_provider);
}

#[test]
fn aws_iam_settings_resolve_from_the_environment_when_explicit_fields_are_absent() {
    let mut overrides = blank_overrides();
    overrides.aws_iam_auth = Some(BoolOrText::Bool(true));
    let mut settings = blank_settings();
    settings.ssl = Some(true);

    let mut env = BTreeMap::new();
    env.insert("REDIS_AWS_IAM_USER_NAME".to_owned(), "env-user".to_owned());
    env.insert(
        "REDIS_AWS_IAM_CACHE_NAME".to_owned(),
        "env-cache".to_owned(),
    );
    env.insert("REDIS_AWS_IAM_REGION".to_owned(), "env-region".to_owned());

    apply_iam_precedence(&overrides, &mut settings, &env)
        .expect("every required aws iam setting is present through the environment");
    assert!(settings.credential_provider);
}

#[test]
fn password_round_trips_through_expose_secret_when_aws_iam_does_not_apply() {
    let overrides = blank_overrides();
    let mut settings = blank_settings();
    settings.password = Some(SecretString::from("kept".to_owned()));

    apply_iam_precedence(&overrides, &mut settings, &BTreeMap::new())
        .expect("no iam provider is configured");

    assert_eq!(
        settings.password.as_ref().map(ExposeSecret::expose_secret),
        Some("kept")
    );
}
