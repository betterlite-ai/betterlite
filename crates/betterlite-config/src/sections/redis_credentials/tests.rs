#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::aws::AwsSettings;
use super::azure::AzureCredential;
use super::{Provider, ProviderWarning};
use crate::loader::ConfigLoader;

fn load(source: &str, environment: BTreeMap<String, String>) -> super::Settings {
    ConfigLoader {
        lookup: environment,
    }
    .load(source)
    .unwrap()
    .sections
    .redis_credentials
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
    let Some(Provider::Gcp(gcp)) = &settings.provider else {
        panic!("expected a gcp provider, got {:?}", settings.provider);
    };
    assert_eq!(gcp.token_ttl_seconds, 3300);
    assert_eq!(settings.warnings, vec![ProviderWarning::GcpOutranksAzure]);
    assert!(settings.environment_snapshot["REDIS_GCP_SERVICE_ACCOUNT"]);
}

#[test]
fn azure_ad_token_must_be_exactly_true_to_enable_azure() {
    let mut environment = BTreeMap::new();
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load("{}", environment);
    assert!(settings.provider.is_none());
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
    assert_eq!(
        matches!(settings.provider, Some(Provider::Azure(_))),
        expect_enabled
    );
}

#[rstest]
#[case(
    Some("client"),
    Some("tenant"),
    Some("secret"),
    AzureCredential::ClientSecret
)]
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
    let Some(Provider::Azure(azure)) = settings.provider else {
        panic!("expected an azure provider");
    };
    assert_eq!(azure.credential, expected);
}

#[test]
fn azure_username_survives_with_provider_active() {
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    environment.insert("REDIS_USERNAME".to_owned(), "app".to_owned());
    let settings = load("{}", environment);
    let Some(Provider::Azure(azure)) = settings.provider else {
        panic!("expected an azure provider");
    };
    assert_eq!(azure.username, Some("app".to_owned()));
    assert_eq!(settings.username, Some("app".to_owned()));
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
    assert!(matches!(settings.provider, Some(Provider::Gcp(_))));
    assert_eq!(settings.warnings, vec![ProviderWarning::GcpOutranksAws]);
}

#[test]
fn azure_outranks_aws_and_warns() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    ssl: true\n";
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "true".to_owned());
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    let settings = load(source, environment);
    assert!(matches!(settings.provider, Some(Provider::Azure(_))));
    assert_eq!(settings.warnings, vec![ProviderWarning::AzureOutranksAws]);
}

#[rstest]
#[case("aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    ssl: true\n")]
#[case("aws_iam_region: eu-west-1\n    aws_iam_cache_name: cache\n    ssl: true\n")]
#[case("aws_iam_region: eu-west-1\n    aws_iam_user_name: app\n    ssl: true\n")]
#[case(
    "aws_iam_user_name: \"\"\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    ssl: true\n"
)]
#[case(
    "aws_iam_user_name: app\n    aws_iam_cache_name: \"\"\n    aws_iam_region: eu-west-1\n    ssl: true\n"
)]
#[case(
    "aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: \"\"\n    ssl: true\n"
)]
fn aws_requires_every_setting(#[case] remaining: &str) {
    let source =
        format!("litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    {remaining}");
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap_err();
}

#[test]
fn aws_empty_region_falls_back_to_environment() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: \"\"\n    ssl: true\n";
    let mut environment = BTreeMap::new();
    environment.insert("AWS_REGION".to_owned(), "eu-west-1".to_owned());
    let settings = load(source, environment);
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert_eq!(aws.region, "eu-west-1");
}

#[rstest]
#[case("true", true)]
#[case("\"1\"", true)]
#[case("yes", true)]
#[case("false", false)]
fn aws_serverless_accepts_text_forms(#[case] value: &str, #[case] expect_serverless: bool) {
    let source = format!(
        "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    aws_iam_serverless: {value}\n    ssl: true\n"
    );
    let settings = load(&source, BTreeMap::new());
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert_eq!(aws.serverless, expect_serverless);
}

#[test]
fn aws_iam_serverless_config_false_overrides_environment_true() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    aws_iam_serverless: false\n    ssl: true\n";
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AWS_IAM_SERVERLESS".to_owned(), "true".to_owned());
    let settings = load(source, environment);
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert!(!aws.serverless);
}

#[test]
fn aws_iam_auth_config_false_overrides_environment_true() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: false\n";
    let mut environment = BTreeMap::new();
    environment.insert("REDIS_AWS_IAM_AUTH".to_owned(), "true".to_owned());
    let settings = load(source, environment);
    assert!(settings.provider.is_none());
}

#[test]
fn aws_requires_tls_via_ssl_flag() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n";
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap_err();
}

#[test]
fn aws_requires_tls_accepts_rediss_url_without_ssl_flag() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: cache\n    aws_iam_region: eu-west-1\n    url: rediss://cache.example:6380\n";
    let settings = load(source, BTreeMap::new());
    assert!(matches!(settings.provider, Some(Provider::Aws(_))));
}

#[test]
fn lowercases_aws_cache_and_requires_tls() {
    let source = "litellm_settings:\n  cache_params:\n    aws_iam_auth: true\n    aws_iam_user_name: app\n    aws_iam_cache_name: Cache.EXAMPLE\n    aws_iam_region: eu-west-1\n    ssl: true\n";
    let settings = load(source, BTreeMap::new());
    let Some(Provider::Aws(aws)) = settings.provider else {
        panic!("expected an aws provider");
    };
    assert_eq!(aws.cache_name, "cache.example");
    assert_eq!(aws.token_ttl_seconds, 900);
}

#[rstest]
#[case("app", false, "cache", "Action=connect&User=app")]
#[case(
    "app",
    true,
    "cache",
    "Action=connect&User=app&ResourceType=ServerlessCache"
)]
#[case("app user", false, "cache", "Action=connect&User=app+user")]
fn connect_query_encodes_user_and_resource_type(
    #[case] user_name: &str,
    #[case] serverless: bool,
    #[case] cache_name: &str,
    #[case] expected: &str,
) {
    let aws = AwsSettings {
        user_name: user_name.to_owned(),
        cache_name: cache_name.to_owned(),
        region: "eu-west-1".to_owned(),
        serverless,
        token_ttl_seconds: 900,
    };
    assert_eq!(aws.connect_query(), expected);
}

#[test]
fn strips_userinfo_from_url_when_provider_is_active() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert(
        "REDIS_URL".to_owned(),
        "redis://user:pass@cache.example:6379".to_owned(),
    );
    let settings = load("{}", environment);
    let url = settings.url.unwrap();
    let exposed = url.expose_secret();
    assert!(!exposed.contains("user"));
    assert!(!exposed.contains("pass"));
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
