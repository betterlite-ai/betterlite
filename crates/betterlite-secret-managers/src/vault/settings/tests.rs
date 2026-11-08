#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::cyberark::transport::ClientCertificate;
use crate::vault::error::VaultInitError;
use crate::vault::settings::{
    DEFAULT_ADDRESS, DEFAULT_MOUNT, DEFAULT_REFRESH_SECONDS, VaultAuth, VaultSettings,
};
use crate::vault::tests::environment;

#[test]
fn defaults_apply_when_only_the_token_is_set() {
    let settings =
        VaultSettings::from_environment(&environment(&[("HCP_VAULT_TOKEN", "token-1")])).unwrap();
    assert_eq!(settings.addr, DEFAULT_ADDRESS);
    assert_eq!(settings.namespace, None);
    assert_eq!(settings.login_namespace, None);
    assert_eq!(settings.secret_namespace, None);
    assert_eq!(settings.mount, DEFAULT_MOUNT);
    assert_eq!(settings.path_prefix, None);
    assert_eq!(
        settings.refresh_interval,
        Duration::from_secs(DEFAULT_REFRESH_SECONDS)
    );
    assert!(matches!(settings.auth, VaultAuth::Token(_)));
}

#[test]
fn empty_variables_count_as_unset() {
    let settings = VaultSettings::from_environment(&environment(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_ADDR", ""),
        ("HCP_VAULT_NAMESPACE", ""),
        ("HCP_VAULT_LOGIN_NAMESPACE", ""),
        ("HCP_VAULT_SECRET_NAMESPACE", ""),
        ("HCP_VAULT_MOUNT_NAME", ""),
        ("HCP_VAULT_PATH_PREFIX", ""),
        ("HCP_VAULT_APPROLE_ROLE_ID", ""),
        ("HCP_VAULT_APPROLE_SECRET_ID", ""),
        ("HCP_VAULT_APPROLE_MOUNT_PATH", ""),
        ("HCP_VAULT_CLIENT_CERT", ""),
        ("HCP_VAULT_CLIENT_KEY", ""),
        ("HCP_VAULT_CERT_ROLE", ""),
        ("HCP_VAULT_REFRESH_INTERVAL", ""),
    ]))
    .unwrap();
    assert_eq!(settings.addr, DEFAULT_ADDRESS);
    assert_eq!(settings.namespace, None);
    assert_eq!(settings.login_namespace, None);
    assert_eq!(settings.secret_namespace, None);
    assert_eq!(settings.mount, DEFAULT_MOUNT);
    assert_eq!(settings.path_prefix, None);
    assert_eq!(
        settings.refresh_interval,
        Duration::from_secs(DEFAULT_REFRESH_SECONDS)
    );
    assert!(matches!(settings.auth, VaultAuth::Token(_)));
}

#[rstest]
#[case::vault_variable_wins(
    &[("HCP_VAULT_REFRESH_INTERVAL", "10"), ("SECRET_MANAGER_REFRESH_INTERVAL", "20")],
    10
)]
#[case::secret_manager_variable_is_the_fallback(&[("SECRET_MANAGER_REFRESH_INTERVAL", "30")], 30)]
#[case::zero_is_used_as_given(&[("HCP_VAULT_REFRESH_INTERVAL", "0")], 0)]
fn refresh_interval_resolves_in_order(#[case] pairs: &[(&str, &str)], #[case] seconds: u64) {
    let mut environment = environment(pairs);
    environment.insert("HCP_VAULT_TOKEN".to_owned(), "token-1".to_owned());
    let settings = VaultSettings::from_environment(&environment).unwrap();
    assert_eq!(settings.refresh_interval, Duration::from_secs(seconds));
}

#[rstest]
#[case::vault_variable("HCP_VAULT_REFRESH_INTERVAL")]
#[case::secret_manager_variable("SECRET_MANAGER_REFRESH_INTERVAL")]
fn invalid_refresh_interval_names_the_variable(#[case] variable: &str) {
    let mut environment = environment(&[(variable, "not-a-number")]);
    environment.insert("HCP_VAULT_TOKEN".to_owned(), "token-1".to_owned());
    let error = VaultSettings::from_environment(&environment).unwrap_err();
    assert!(matches!(
        error,
        VaultInitError::RefreshInterval {
            source: betterlite_env::EnvError::Number { name }
        } if name == variable
    ));
}

#[test]
fn missing_credentials_are_refused() {
    let error = VaultSettings::from_environment(&environment(&[])).unwrap_err();
    assert!(matches!(error, VaultInitError::MissingCredentials));
}

#[test]
fn approle_beats_tls_and_the_token() {
    let settings = VaultSettings::from_environment(&environment(&[
        ("HCP_VAULT_APPROLE_ROLE_ID", "role-1"),
        ("HCP_VAULT_APPROLE_SECRET_ID", "secret-1"),
        ("HCP_VAULT_CLIENT_CERT", "/certs/client.crt"),
        ("HCP_VAULT_CLIENT_KEY", "/certs/client.key"),
        ("HCP_VAULT_TOKEN", "token-1"),
    ]))
    .unwrap();
    match settings.auth {
        VaultAuth::AppRole {
            role_id,
            secret_id,
            mount_path,
        } => {
            assert_eq!(role_id.expose_secret(), "role-1");
            assert_eq!(secret_id.expose_secret(), "secret-1");
            assert_eq!(mount_path, "approle");
        }
        VaultAuth::TlsCert { .. } | VaultAuth::Token(_) => panic!("AppRole must win"),
    }
}

#[test]
fn tls_beats_the_token() {
    let settings = VaultSettings::from_environment(&environment(&[
        ("HCP_VAULT_CLIENT_CERT", "/certs/client.crt"),
        ("HCP_VAULT_CLIENT_KEY", "/certs/client.key"),
        ("HCP_VAULT_CERT_ROLE", "vault-role"),
        ("HCP_VAULT_TOKEN", "token-1"),
    ]))
    .unwrap();
    match settings.auth {
        VaultAuth::TlsCert {
            client_certificate,
            role,
        } => {
            assert_eq!(
                client_certificate,
                ClientCertificate {
                    certificate_path: "/certs/client.crt".to_owned(),
                    key_path: "/certs/client.key".to_owned(),
                }
            );
            assert_eq!(role.as_deref(), Some("vault-role"));
        }
        VaultAuth::AppRole { .. } | VaultAuth::Token(_) => panic!("TLS cert must win"),
    }
}

#[rstest]
#[case::role_without_secret(&[("HCP_VAULT_APPROLE_ROLE_ID", "role-1")])]
#[case::secret_without_role(&[("HCP_VAULT_APPROLE_SECRET_ID", "secret-1")])]
#[case::certificate_without_key(&[("HCP_VAULT_CLIENT_CERT", "/certs/client.crt")])]
#[case::key_without_certificate(&[("HCP_VAULT_CLIENT_KEY", "/certs/client.key")])]
fn half_a_credential_pair_counts_for_nothing(#[case] pairs: &[(&str, &str)]) {
    let mut environment = environment(pairs);
    environment.insert("HCP_VAULT_TOKEN".to_owned(), "token-1".to_owned());
    let settings = VaultSettings::from_environment(&environment).unwrap();
    match settings.auth {
        VaultAuth::Token(token) => assert_eq!(token.expose_secret(), "token-1"),
        VaultAuth::AppRole { .. } | VaultAuth::TlsCert { .. } => {
            panic!("an incomplete pair must fall through")
        }
    }
}

#[test]
fn an_incomplete_pair_alone_has_no_credentials() {
    let error = VaultSettings::from_environment(&environment(&[(
        "HCP_VAULT_CLIENT_CERT",
        "/certs/client.crt",
    )]))
    .unwrap_err();
    assert!(matches!(error, VaultInitError::MissingCredentials));
}

#[test]
fn approle_mount_path_is_used_as_given() {
    let settings = VaultSettings::from_environment(&environment(&[
        ("HCP_VAULT_APPROLE_ROLE_ID", "role-1"),
        ("HCP_VAULT_APPROLE_SECRET_ID", "secret-1"),
        ("HCP_VAULT_APPROLE_MOUNT_PATH", " /custom-approle/ "),
    ]))
    .unwrap();
    match settings.auth {
        VaultAuth::AppRole { mount_path, .. } => {
            assert_eq!(mount_path, " /custom-approle/ ");
        }
        VaultAuth::TlsCert { .. } | VaultAuth::Token(_) => panic!("AppRole must win"),
    }
}

#[test]
fn a_tls_certificate_may_have_no_role() {
    let settings = VaultSettings::from_environment(&environment(&[
        ("HCP_VAULT_CLIENT_CERT", "/certs/client.crt"),
        ("HCP_VAULT_CLIENT_KEY", "/certs/client.key"),
    ]))
    .unwrap();
    match settings.auth {
        VaultAuth::TlsCert { role, .. } => assert_eq!(role, None),
        VaultAuth::AppRole { .. } | VaultAuth::Token(_) => panic!("TLS cert must win"),
    }
}

#[test]
fn debug_output_hides_every_credential() {
    let settings = VaultSettings::from_environment(&environment(&[
        ("HCP_VAULT_APPROLE_ROLE_ID", "role-hidden"),
        ("HCP_VAULT_APPROLE_SECRET_ID", "secret-hidden"),
        ("HCP_VAULT_CLIENT_CERT", "/certs/client.crt"),
        ("HCP_VAULT_CLIENT_KEY", "/certs/client.key"),
        ("HCP_VAULT_TOKEN", "token-hidden"),
    ]))
    .unwrap();
    let debug = format!("{settings:?}");
    assert!(!debug.contains("role-hidden"));
    assert!(!debug.contains("secret-hidden"));
    assert!(!debug.contains("token-hidden"));
}

#[test]
fn debug_output_hides_the_direct_token() {
    let settings =
        VaultSettings::from_environment(&environment(&[("HCP_VAULT_TOKEN", "token-hidden")]))
            .unwrap();
    assert!(!format!("{settings:?}").contains("token-hidden"));
}
