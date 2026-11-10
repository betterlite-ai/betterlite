#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::manager::ManagerState;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use rstest::rstest;
use secrecy::SecretString;

use super::{PTU_COST_ATTRIBUTION_ENV_VAR, load_ptu_cost_flag};

struct NoHttp;

impl OidcHttp for NoHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no network in this test".to_owned(),
        })
    }
}

struct NoAzure;

impl AzureOidcTokens for NoAzure {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

struct StillClock;

impl OidcClock for StillClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

struct VaultReply(Option<&'static str>);

impl SyncSecretReader for VaultReply {
    fn sync_read_secret(&self, _secret_name: &str) -> Option<SecretString> {
        self.0.map(SecretString::from)
    }
}

fn lookup(manager: Option<ManagerState>) -> SecretLookup {
    SecretLookup {
        manager,
        oidc_http: Arc::new(NoHttp),
        azure_tokens: Arc::new(NoAzure),
        clock: Arc::new(StillClock),
        oidc_cache: OidcTokenCache::default(),
    }
}

fn vault_manager(
    reply: Option<&'static str>,
    access_mode: SecretAccessMode,
    hosted_keys: Option<Vec<String>>,
) -> ManagerState {
    ManagerState {
        client: SecretManagerClient::Vault(Arc::new(VaultReply(reply))),
        settings: Some(SecretManagerSettings::default()),
        system: Some(KeyManagementSystem::HashicorpVault),
        access_mode,
        hosted_keys,
    }
}

fn environment(value: Option<&str>) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::from([
        ("HOME".to_owned(), "/home/user".to_owned()),
        ("LITELLM_MASTER_KEY".to_owned(), "sk-test".to_owned()),
        ("PATH".to_owned(), "/usr/local/bin".to_owned()),
    ]);
    if let Some(text) = value {
        environment.insert(PTU_COST_ATTRIBUTION_ENV_VAR.to_owned(), text.to_owned());
    }
    environment
}

#[rstest]
#[case::absent(None, false)]
#[case::empty(Some(""), false)]
#[case::exact_true(Some("true"), true)]
#[case::upper_case(Some("TRUE"), true)]
#[case::padded(Some(" true "), true)]
#[case::exact_false(Some("false"), false)]
#[case::upper_false(Some("FALSE"), false)]
#[case::one(Some("1"), false)]
#[case::yes(Some("yes"), false)]
#[case::on(Some("on"), false)]
#[case::other(Some("enabled"), false)]
fn loads_ptu_cost_flag(#[case] value: Option<&str>, #[case] expected: bool) {
    assert_eq!(
        PTU_COST_ATTRIBUTION_ENV_VAR,
        "LITELLM_ENABLE_PTU_COST_ATTRIBUTION"
    );
    assert_eq!(
        load_ptu_cost_flag(&lookup(None), &environment(value)),
        expected
    );
}

#[rstest]
#[case::manager_true_env_absent(Some("true"), None, SecretAccessMode::ReadOnly, true)]
#[case::manager_true_read_and_write(Some("true"), None, SecretAccessMode::ReadAndWrite, true)]
#[case::manager_true_over_env_false(Some("true"), Some("false"), SecretAccessMode::ReadOnly, true)]
#[case::manager_false_over_env_true(Some("false"), Some("true"), SecretAccessMode::ReadOnly, false)]
#[case::manager_other_text(Some("enabled"), Some("true"), SecretAccessMode::ReadOnly, false)]
#[case::manager_missing_falls_back_to_env(None, Some("true"), SecretAccessMode::ReadOnly, true)]
#[case::manager_missing_and_env_absent(None, None, SecretAccessMode::ReadOnly, false)]
#[case::write_only_ignores_manager(Some("true"), None, SecretAccessMode::WriteOnly, false)]
#[case::write_only_reads_env(Some("false"), Some("true"), SecretAccessMode::WriteOnly, true)]
fn loads_ptu_cost_flag_from_a_configured_manager(
    #[case] manager_value: Option<&'static str>,
    #[case] env_value: Option<&str>,
    #[case] access_mode: SecretAccessMode,
    #[case] expected: bool,
) {
    let manager = vault_manager(manager_value, access_mode, None);
    assert_eq!(
        load_ptu_cost_flag(&lookup(Some(manager)), &environment(env_value)),
        expected
    );
}

#[rstest]
#[case::name_hosted(vec![PTU_COST_ATTRIBUTION_ENV_VAR.to_owned()], true)]
#[case::name_not_hosted(vec!["OTHER_KEY".to_owned()], false)]
fn loads_ptu_cost_flag_honours_the_hosted_keys(
    #[case] hosted_keys: Vec<String>,
    #[case] expected: bool,
) {
    let manager = vault_manager(Some("true"), SecretAccessMode::ReadOnly, Some(hosted_keys));
    assert_eq!(
        load_ptu_cost_flag(&lookup(Some(manager)), &environment(None)),
        expected
    );
}

#[test]
fn loads_ptu_cost_flag_needs_manager_settings() {
    let mut manager = vault_manager(Some("true"), SecretAccessMode::ReadOnly, None);
    manager.settings = None;
    assert!(!load_ptu_cost_flag(
        &lookup(Some(manager)),
        &environment(None)
    ));
}
