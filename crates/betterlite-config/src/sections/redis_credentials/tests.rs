#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::azure::AzureCredential;
use super::{Provider, ProviderWarning};
use crate::loader::ConfigLoader;

fn load(source: &str, environment: BTreeMap<String, String>) -> super::Settings {
    ConfigLoader { lookup: environment }.load(source).unwrap().sections.redis_credentials
}

#[test]
fn resolves_gcp_from_environment_with_precedence_over_azure() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    let settings = load("{}", environment);
    assert_eq!(settings.provider, Some(Provider::Gcp));
    assert_eq!(settings.gcp.as_ref().map(|value| value.token_ttl_seconds), Some(3300));
    assert!(settings.azure.is_none());
    assert_eq!(settings.warnings, vec![ProviderWarning::GcpOutranksAzure]);
    assert!(settings.environment_snapshot["REDIS_GCP_SERVICE_ACCOUNT"]);
}

#[test]
fn azure_ad_token_must_be_exactly_true_to_enable_azure() {
    let mut environment = BTreeMap::new();
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load("{}", environment);
    assert!(settings.azure.is_none());
    assert_eq!(settings.provider, None);
}

#[rstest]
#[case("true", true)]
#[case("TRUE", true)]
#[case("false", false)]
#[case("client-id-only", false)]
fn azure_ad_token_gates_on_strict_true(#[case] token: &str, #[case] expect_enabled: bool) {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), token.to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load("{}", environment);
    assert_eq!(settings.azure.is_some(), expect_enabled);
}

#[rstest]
#[case(Some("client"), Some("tenant"), Some("secret"), AzureCredential::ClientSecret)]
#[case(Some("client"), None, None, AzureCredential::ManagedIdentity)]
#[case(None, None, None, AzureCredential::Default)]
fn selects_azure_credential_kind_from_config(
    #[case] client_id: Option<&str>,
    #[case] tenant_id: Option<&str>,
    #[case] client_secret: Option<&str>,
    #[case] expected: AzureCredential,
) {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    if let Some(value) = client_id {
        environment.insert("AZURE_CLIENT_ID".to_owned(), value.to_owned());
    }
    if let Some(value) = tenant_id {
        environment.insert("AZURE_TENANT_ID".to_owned(), value.to_owned());
    }
    if let Some(value) = client_secret {
        environment.insert("AZURE_CLIENT_SECRET".to_owned(), value.to_owned());
    }
    let settings = load("{}", environment);
    assert_eq!(settings.azure.unwrap().credential, expected);
}

#[test]
fn gcp_outranks_aws_and_warns() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert("REDIS_AWS_IAM_AUTH".to_owned(), "true".to_owned());
    let settings = load("{}", environment);
    assert_eq!(settings.provider, Some(Provider::Gcp));
    assert_eq!(settings.warnings, vec![ProviderWarning::GcpOutranksAws]);
}

#[test]
fn azure_outranks_aws_and_warns() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    ssl: true\n";
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load(source, environment);
    assert_eq!(settings.provider, Some(Provider::Azure));
    assert_eq!(settings.warnings, vec![ProviderWarning::AzureOutranksAws]);
}

#[rstest]
#[case("aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    ssl: true\n")]
#[case("aws_iam_region: eu-west-1\n    aws_iam_cache_name: cache\n    ssl: true\n")]
#[case("aws_iam_region: eu-west-1\n    aws_iam_user_name: app\n    ssl: true\n")]
fn aws_requires_every_setting(#[case] remaining: &str) {
    let source = format!("litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    {remaining}");
    let result = ConfigLoader { lookup: BTreeMap::new() }.load(&source);
    assert!(result.is_err());
}

#[rstest]
#[case("true", true)]
#[case("1", true)]
#[case("yes", true)]
#[case("false", false)]
fn aws_serverless_accepts_text_forms(#[case] value: &str, #[case] expect_serverless: bool) {
    let source = format!(
        "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    aws_iam_serverless: {value}\n    ssl: true\n"
    );
    let settings = load(&source, BTreeMap::new());
    assert_eq!(settings.aws.unwrap().serverless, expect_serverless);
}

#[test]
fn aws_requires_tls_via_ssl_flag() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n";
    let result = ConfigLoader { lookup: BTreeMap::new() }.load(source);
    assert!(result.is_err());
}

#[test]
fn aws_requires_tls_accepts_rediss_url_without_ssl_flag() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    url: rediss://cache.example:6380\n";
    let settings = load(source, BTreeMap::new());
    assert!(settings.aws.is_some());
}

#[test]
fn lowercases_aws_cache_and_requires_tls() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: Cache.EXAMPLE\n    aws_iam_region: eu-west-1\n    ssl: true\n";
    let settings = load(source, BTreeMap::new());
    let aws = settings.aws.unwrap();
    assert_eq!(aws.cache_name, "cache.example");
    assert_eq!(aws.token_ttl_seconds, 900);
}

#[test]
fn strips_userinfo_from_url_when_provider_is_active() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert("REDIS_URL".to_owned(), "redis://user:pass@cache.example:6379".to_owned());
    let settings = load("{}", environment);
    let url = settings.url.unwrap();
    assert!(!url.contains("user"));
    assert!(!url.contains("pass"));
}

#[test]
fn keeps_username_and_password_when_no_provider_is_active() {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_USERNAME".to_owned(), "app".to_owned());
    environment.insert("REDIS_PASSWORD".to_owned(), "secret".to_owned());
    let settings = load("{}", environment);
    assert_eq!(settings.username, Some("app".to_owned()));
    assert!(settings.password.is_some());
}

#[test]
fn environment_snapshot_reports_unset_names_as_false() {
    let settings = load("{}", BTreeMap::new());
    assert!(!settings.environment_snapshot["REDIS_GCP_SERVICE_ACCOUNT"]);
    assert!(!settings.environment_snapshot["AWS_REGION"]);
}
