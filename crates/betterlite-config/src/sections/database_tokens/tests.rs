#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_db::database_tokens::provider::{
    AzureProviderCache, AzureProviderFactory, AzureTokenProvider, RdsTokenProvider,
};
use betterlite_db::database_tokens::settings::POOLED_MARKER_ENV_VAR;
use betterlite_db::database_tokens::strategy::{TokenProviders, resolve};
use betterlite_db::rds_credentials::RdsEndpoint;
use betterlite_db::rds_credentials::error::RdsCredentialsError;
use rstest::rstest;
use secrecy::SecretString;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn load(source: &str, environment: &[(&str, &str)]) -> Settings {
    load_result(source, environment)
        .expect("the config loads")
        .sections
        .database_tokens
}

fn load_result(source: &str, environment: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }.load(source)
}

#[test]
fn flags_default_false() {
    let settings = load("{}", &[]);
    assert!(!settings.toggles.iam_token_db_auth);
    assert!(!settings.toggles.azure_postgresql_auth);
    assert!(!settings.pooled_database_url);
}

#[rstest]
#[case::one("1")]
#[case::on("on")]
#[case::true_value("true")]
#[case::yes("yes")]
#[case::padded_and_upper(" True ")]
#[case::zero("0")]
#[case::off("off")]
#[case::false_value("false")]
#[case::no("no")]
#[case::empty("")]
fn flags_all_strict_spellings(#[case] value: &str) {
    let expected = matches!(value, "1" | "on" | "true" | "yes" | " True ");
    let iam = load("{}", &[("IAM_TOKEN_DB_AUTH", value)]);
    assert_eq!(iam.toggles.iam_token_db_auth, expected, "{value:?}");
    assert!(!iam.toggles.azure_postgresql_auth);
    let azure = load("{}", &[("AZURE_POSTGRESQL_AUTH", value)]);
    assert_eq!(azure.toggles.azure_postgresql_auth, expected, "{value:?}");
    assert!(!azure.toggles.iam_token_db_auth);
}

#[rstest]
#[case::t("t")]
#[case::y("y")]
#[case::f("f")]
#[case::n("n")]
#[case::maybe("maybe")]
fn invalid_flag_stops_loading(#[case] value: &str) {
    let iam = load_result("{}", &[("IAM_TOKEN_DB_AUTH", value)]).expect_err("the flag is invalid");
    assert_eq!(iam.category(), "environment");
    let azure =
        load_result("{}", &[("AZURE_POSTGRESQL_AUTH", value)]).expect_err("the flag is invalid");
    assert_eq!(azure.category(), "environment");
}

struct NoRds;

impl RdsTokenProvider for NoRds {
    fn mint(&self, _endpoint: &RdsEndpoint<'_>) -> Result<SecretString, RdsCredentialsError> {
        Err(RdsCredentialsError::NoCredentials)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("no credential in the config test")]
struct NoAzureError;

struct NoAzureProvider;

impl AzureTokenProvider for NoAzureProvider {
    type Error = NoAzureError;

    fn token(&self) -> Result<SecretString, NoAzureError> {
        Err(NoAzureError)
    }
}

struct NoAzure {
    builds: Arc<AtomicUsize>,
}

impl AzureProviderFactory for NoAzure {
    type Provider = NoAzureProvider;
    type Error = NoAzureError;

    fn build(&self, _scope: &str) -> Result<Arc<NoAzureProvider>, NoAzureError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        Err(NoAzureError)
    }
}

#[test]
fn both_flags_conflict() {
    let settings = load(
        "{}",
        &[
            ("IAM_TOKEN_DB_AUTH", "true"),
            ("AZURE_POSTGRESQL_AUTH", "yes"),
        ],
    );
    assert!(settings.toggles.iam_token_db_auth);
    assert!(settings.toggles.azure_postgresql_auth);
    let builds = Arc::new(AtomicUsize::new(0));
    let providers = TokenProviders {
        rds: Arc::new(NoRds),
        azure: Arc::new(AzureProviderCache::new(Arc::new(NoAzure {
            builds: Arc::clone(&builds),
        }))),
    };
    let Err(error) = resolve(settings.toggles, &providers) else {
        panic!("both enabled flags must conflict");
    };
    assert_eq!(
        error.to_string(),
        "IAM_TOKEN_DB_AUTH and AZURE_POSTGRESQL_AUTH are both enabled, but the database password can only come from one token source. Keep IAM_TOKEN_DB_AUTH for AWS RDS IAM auth, or AZURE_POSTGRESQL_AUTH for Azure Database for PostgreSQL with Microsoft Entra ID, and unset the other one."
    );
    assert_eq!(builds.load(Ordering::SeqCst), 0);
}

#[rstest]
#[case::exact_lower(POOLED_MARKER_ENV_VAR, "true", true)]
#[case::title_case(POOLED_MARKER_ENV_VAR, "True", false)]
#[case::upper_case(POOLED_MARKER_ENV_VAR, "TRUE", false)]
#[case::one(POOLED_MARKER_ENV_VAR, "1", false)]
#[case::yes(POOLED_MARKER_ENV_VAR, "yes", false)]
#[case::empty(POOLED_MARKER_ENV_VAR, "", false)]
#[case::lowercase_name("litellm_pgbouncer_pooled_database_url", "true", false)]
fn pooled_marker_exact_true_only(#[case] name: &str, #[case] value: &str, #[case] expected: bool) {
    let settings = load("{}", &[(name, value)]);
    assert_eq!(settings.pooled_database_url, expected, "{name}={value:?}");
}
