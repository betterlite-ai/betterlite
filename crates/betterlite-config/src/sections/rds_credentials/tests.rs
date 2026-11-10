#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Settings {
    load_result(source, environment).expect("the config loads")
}

fn load_result(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.rds_credentials)
}

#[test]
fn aws_env_defaults() {
    let settings = load(
        "{}",
        &[
            ("AWS_REGION_NAME", "eu-west-1"),
            ("AWS_ACCESS_KEY_ID", "AK-ENV"),
            ("AWS_SECRET_ACCESS_KEY", "SK-ENV"),
            ("AWS_SESSION_NAME", "sess-env"),
            ("AWS_PROFILE_NAME", "prof-env"),
            ("AWS_ROLE_NAME", "arn:env"),
            ("AWS_WEB_IDENTITY_TOKEN", "token-env"),
        ],
    );
    let credentials = settings.credentials;
    assert_eq!(credentials.region_name.as_deref(), Some("eu-west-1"));
    assert_eq!(
        credentials
            .access_key_id
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("AK-ENV")
    );
    assert_eq!(
        credentials
            .secret_access_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("SK-ENV")
    );
    assert_eq!(credentials.session_name.as_deref(), Some("sess-env"));
    assert_eq!(credentials.profile_name.as_deref(), Some("prof-env"));
    assert_eq!(credentials.role_name.as_deref(), Some("arn:env"));
    assert_eq!(
        credentials
            .web_identity_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("token-env")
    );
}

#[rstest]
#[case::alias_absent(&[("AWS_ROLE_ARN", "arn:alias")], Some("arn:alias"))]
#[case::explicit_empty(&[("AWS_ROLE_NAME", ""), ("AWS_ROLE_ARN", "arn:alias")], Some(""))]
#[case::explicit_name(&[("AWS_ROLE_NAME", "named"), ("AWS_ROLE_ARN", "arn:alias")], Some("named"))]
#[case::both_absent(&[], None)]
fn role_alias_absent_vs_empty(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = load("{}", environment);
    assert_eq!(settings.credentials.role_name.as_deref(), expected);
}

#[rstest]
#[case::alias_absent(&[("AWS_WEB_IDENTITY_TOKEN_FILE", "/token")], Some("/token"))]
#[case::explicit_empty(&[("AWS_WEB_IDENTITY_TOKEN", ""), ("AWS_WEB_IDENTITY_TOKEN_FILE", "/token")], Some(""))]
#[case::explicit_token(&[("AWS_WEB_IDENTITY_TOKEN", "token"), ("AWS_WEB_IDENTITY_TOKEN_FILE", "/token")], Some("token"))]
#[case::both_absent(&[], None)]
fn web_identity_alias_absent_vs_empty(
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = load("{}", environment);
    assert_eq!(
        settings
            .credentials
            .web_identity_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        expected
    );
}

#[test]
fn aws_secret_reference_resolution() {
    let source = "general_settings:\n  aws_access_key_id: os.environ/AWS_KEY_REF\n  aws_secret_access_key: os.environ/AWS_SECRET_REF\n  aws_web_identity_token: os.environ/AWS_TOKEN_REF\n";
    let settings = load(
        source,
        &[
            ("AWS_KEY_REF", "AK-REF"),
            ("AWS_SECRET_REF", "SK-REF"),
            ("AWS_TOKEN_REF", "/token"),
        ],
    );
    let credentials = settings.credentials;
    assert_eq!(
        credentials
            .access_key_id
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("AK-REF")
    );
    assert_eq!(
        credentials
            .secret_access_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("SK-REF")
    );
    assert_eq!(
        credentials
            .web_identity_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("/token")
    );
}

#[test]
fn unset_secret_reference_is_a_missing_environment_error() {
    let source = "general_settings:\n  aws_access_key_id: os.environ/MISSING_REF\n  aws_region_name: us-east-2\n";
    let error = load_result(source, &[("AWS_ACCESS_KEY_ID", "AK-ENV")])
        .expect_err("an unset reference fails instead of taking the environment");
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("expected a missing environment error, got {error}");
    };
    assert_eq!(variable, "MISSING_REF");
    assert_eq!(path, "general_settings.aws_access_key_id");
}

#[test]
fn absent_keys_fall_back_to_the_environment_names() {
    let settings = load("{}", &[("AWS_ACCESS_KEY_ID", "AK-ENV")]);
    assert_eq!(
        settings
            .credentials
            .access_key_id
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("AK-ENV")
    );
}

#[test]
fn configured_values_beat_the_environment() {
    let source =
        "general_settings:\n  aws_region_name: eu-west-1\n  aws_role_name: arn:configured\n";
    let settings = load(
        source,
        &[
            ("AWS_REGION_NAME", "us-east-1"),
            ("AWS_ROLE_NAME", "arn:env"),
        ],
    );
    assert_eq!(
        settings.credentials.region_name.as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        settings.credentials.role_name.as_deref(),
        Some("arn:configured")
    );
}

#[test]
fn region_required_before_sdk_defaults() {
    let sdk_defaults = load(
        "{}",
        &[
            ("AWS_DEFAULT_REGION", "eu-central-1"),
            ("AWS_PROFILE", "prod"),
        ],
    );
    assert_eq!(sdk_defaults.credentials.region_name, None);
    let standard = load("{}", &[("AWS_REGION", "us-east-2")]);
    assert_eq!(standard.credentials.region_name, None);
    let custom = load("{}", &[("AWS_REGION_NAME", "us-east-1")]);
    assert_eq!(custom.credentials.region_name.as_deref(), Some("us-east-1"));
}

#[test]
fn settings_debug_hides_the_secret_values() {
    let settings = load(
        "general_settings:\n  aws_access_key_id: marker-access\n  aws_secret_access_key: marker-secret\n  aws_web_identity_token: marker-token\n",
        &[],
    );
    let debug = format!("{settings:?}");
    assert!(!debug.contains("marker-access"));
    assert!(!debug.contains("marker-secret"));
    assert!(!debug.contains("marker-token"));
}
