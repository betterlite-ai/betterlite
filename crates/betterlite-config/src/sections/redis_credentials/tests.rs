#![cfg(test)]

use std::collections::BTreeMap;

use super::Provider;
use super::azure::AzureCredential;
use crate::{loader::ConfigLoader, loader::EnvironmentLookup as _};

#[test]
fn resolves_gcp_from_environment_with_precedence() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "REDIS_GCP_SERVICE_ACCOUNT".to_owned(),
        "projects/-/serviceAccounts/redis".to_owned(),
    );
    environment.insert("REDIS_AZURE_AD_TOKEN".to_owned(), "token".to_owned());
    let config = ConfigLoader {
        lookup: environment,
    }
    .load("{}")
    .unwrap();
    let settings = &config.sections.redis_credentials;
    assert_eq!(settings.provider, Some(Provider::Gcp));
    assert_eq!(
        settings.gcp.as_ref().map(|value| value.token_ttl_seconds),
        Some(3300)
    );
    assert!(settings.environment_snapshot["REDIS_GCP_SERVICE_ACCOUNT"]);
}

#[test]
fn selects_azure_credential_kind_from_config() {
    let mut environment = BTreeMap::new();
    environment.insert("AZURE_CLIENT_ID".to_owned(), "client".to_owned());
    environment.insert("AZURE_TENANT_ID".to_owned(), "tenant".to_owned());
    environment.insert("AZURE_CLIENT_SECRET".to_owned(), "secret".to_owned());
    let config = ConfigLoader {
        lookup: environment,
    }
    .load("{}")
    .unwrap();
    assert_eq!(
        config.sections.redis_credentials.azure.unwrap().credential,
        AzureCredential::ClientSecret
    );
}

#[test]
fn lowercases_aws_cache_and_requires_tls() {
    let source = "litellm_settings:\n  aws_iam_auth: true\n  aws_iam_user_name: app\n  aws_iam_cache_name: Cache.EXAMPLE\n  aws_iam_region: eu-west-1\n  ssl: true\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap();
    let settings = config.sections.redis_credentials.aws.unwrap();
    assert_eq!(settings.cache_name, "cache.example");
    assert_eq!(settings.token_ttl_seconds, 900);
}

#[test]
fn environment_lookup_is_snapshot_based() {
    let lookup = BTreeMap::from([(String::from("AWS_REGION"), String::from("us-east-1"))]);
    assert_eq!(lookup.lookup("AWS_REGION"), Some(String::from("us-east-1")));
}
