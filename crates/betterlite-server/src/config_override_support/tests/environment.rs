#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup as _;

use super::object;
use crate::config_override_support::environment::{RuntimeEnvironment, project_environment};
use crate::config_override_support::models::OverrideKind;

#[test]
fn projects_mapped_values_and_removes_empty_or_missing_ones() {
    let batch = project_environment(
        OverrideKind::HashicorpVault,
        &object(
            r#"{
                "vault_addr": "https://vault.example.com",
                "vault_token": "",
                "vault_namespace": null,
                "client_cert": "cert.pem"
            }"#,
        ),
    );
    assert_eq!(batch.changes.len(), 13);
    assert_eq!(
        batch.changes.get("HCP_VAULT_ADDR"),
        Some(&Some("https://vault.example.com".to_owned()))
    );
    assert_eq!(
        batch.changes.get("HCP_VAULT_CLIENT_CERT"),
        Some(&Some("cert.pem".to_owned()))
    );
    assert_eq!(batch.changes.get("HCP_VAULT_TOKEN"), Some(&None));
    assert_eq!(batch.changes.get("HCP_VAULT_NAMESPACE"), Some(&None));
    assert_eq!(batch.changes.get("HCP_VAULT_APPROLE_ROLE_ID"), Some(&None));
}

#[test]
fn stringifies_the_other_values_the_model_can_carry() {
    let vault = project_environment(
        OverrideKind::HashicorpVault,
        &object(
            r#"{
                "approle_role_id": 7,
                "vault_mount_name": true,
                "vault_path_prefix": ["a", "b"]
            }"#,
        ),
    );
    assert_eq!(
        vault.changes.get("HCP_VAULT_APPROLE_ROLE_ID"),
        Some(&Some("7".to_owned()))
    );
    assert_eq!(
        vault.changes.get("HCP_VAULT_MOUNT_NAME"),
        Some(&Some("true".to_owned()))
    );
    assert_eq!(
        vault.changes.get("HCP_VAULT_PATH_PREFIX"),
        Some(&Some("[\"a\",\"b\"]".to_owned()))
    );
    let cyberark = project_environment(
        OverrideKind::CyberArk,
        &object(r#"{"ssl_verify": false, "refresh_interval": 60}"#),
    );
    assert_eq!(cyberark.changes.len(), 8);
    assert_eq!(
        cyberark.changes.get("CYBERARK_SSL_VERIFY"),
        Some(&Some("false".to_owned()))
    );
    assert_eq!(
        cyberark.changes.get("CYBERARK_REFRESH_INTERVAL"),
        Some(&Some("60".to_owned()))
    );
    assert_eq!(cyberark.changes.get("CYBERARK_ACCOUNT"), Some(&None));
}

#[test]
fn applies_batches_over_boot_values_without_changing_earlier_snapshots() {
    let boot = [
        ("HCP_VAULT_ADDR".to_owned(), "https://boot".to_owned()),
        ("HCP_VAULT_TOKEN".to_owned(), "boot-token".to_owned()),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    let mut environment = RuntimeEnvironment {
        boot,
        overlay: BTreeMap::new(),
    };
    let early = environment.snapshot();
    environment.apply(&project_environment(
        OverrideKind::HashicorpVault,
        &object(r#"{"vault_addr": "https://runtime", "vault_token": null}"#),
    ));
    let late = environment.snapshot();
    assert_eq!(
        early.values,
        BTreeMap::from([
            ("HCP_VAULT_ADDR".to_owned(), "https://boot".to_owned()),
            ("HCP_VAULT_TOKEN".to_owned(), "boot-token".to_owned()),
        ])
    );
    assert_eq!(
        late.values,
        BTreeMap::from([("HCP_VAULT_ADDR".to_owned(), "https://runtime".to_owned())])
    );
    assert_eq!(
        environment.lookup("HCP_VAULT_ADDR").as_deref(),
        Some("https://runtime")
    );
    assert_eq!(environment.lookup("HCP_VAULT_TOKEN"), None);
    assert_eq!(
        early.lookup("HCP_VAULT_TOKEN").as_deref(),
        Some("boot-token")
    );
}
