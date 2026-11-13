#![cfg(test)]

use std::sync::Arc;

use rstest::rstest;

use crate::vault::HashicorpSecretManager;
use crate::vault::target::{VaultSecretParams, VaultSecretSettings};
use crate::vault::tests::{FakeTransport, scripted};

const TOKEN: &[(&str, &str)] = &[("HCP_VAULT_TOKEN", "token-1")];

fn manager(pairs: &[(&str, &str)]) -> HashicorpSecretManager<Arc<FakeTransport>> {
    let (manager, _transport) = scripted(pairs, &[]);
    manager
}

fn default_settings() -> VaultSecretSettings {
    VaultSecretSettings::default()
}

#[test]
fn defaults_build_the_kv_v2_url() {
    let manager = manager(TOKEN);
    assert_eq!(
        manager
            .get_url("OPENAI_API_KEY", &default_settings())
            .unwrap(),
        "http://127.0.0.1:8200/v1/secret/data/OPENAI_API_KEY"
    );
}

#[test]
fn a_global_namespace_goes_into_the_url() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
    ]);
    assert_eq!(
        manager
            .get_url("OPENAI_API_KEY", &default_settings())
            .unwrap(),
        "http://127.0.0.1:8200/v1/admin/secret/data/OPENAI_API_KEY"
    );
}

#[test]
fn the_secret_namespace_override_wins_over_the_global_one() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
        ("HCP_VAULT_SECRET_NAMESPACE", "teams/team-a"),
    ]);
    assert_eq!(
        manager
            .get_url("OPENAI_API_KEY", &default_settings())
            .unwrap(),
        "http://127.0.0.1:8200/v1/teams/team-a/secret/data/OPENAI_API_KEY"
    );
}

#[test]
fn the_login_namespace_never_changes_the_secret_url() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
        ("HCP_VAULT_LOGIN_NAMESPACE", "root"),
    ]);
    assert_eq!(manager.vault_login_namespace(), Some("root"));
    assert_eq!(manager.vault_secret_namespace(), Some("admin"));
    assert_eq!(
        manager
            .get_url("OPENAI_API_KEY", &default_settings())
            .unwrap(),
        "http://127.0.0.1:8200/v1/admin/secret/data/OPENAI_API_KEY"
    );
}

#[test]
fn a_custom_mount_and_prefix_are_trimmed_of_whitespace_and_slashes() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_MOUNT_NAME", " /kv-prod/ "),
        ("HCP_VAULT_PATH_PREFIX", " /virtual-keys/ "),
    ]);
    assert_eq!(
        manager.get_url("DB_PASSWORD", &default_settings()).unwrap(),
        "http://127.0.0.1:8200/v1/kv-prod/data/virtual-keys/DB_PASSWORD"
    );
}

#[test]
fn a_trailing_slash_on_the_address_stays() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_ADDR", "http://vault.test:8200/"),
    ]);
    assert_eq!(
        manager
            .get_url("OPENAI_API_KEY", &default_settings())
            .unwrap(),
        "http://vault.test:8200//v1/secret/data/OPENAI_API_KEY"
    );
}

#[test]
fn the_secret_name_goes_in_raw() {
    let manager = manager(TOKEN);
    assert_eq!(
        manager
            .get_url("apps/db+prod", &default_settings())
            .unwrap(),
        "http://127.0.0.1:8200/v1/secret/data/apps/db+prod"
    );
}

#[test]
fn settings_overrides_win_over_the_manager_values() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
    ]);
    let settings = VaultSecretSettings {
        namespace: Some("teams/team-b".to_owned()),
        mount: Some("kv-prod".to_owned()),
        path_prefix: Some("virtual-keys".to_owned()),
        data: None,
    };
    assert_eq!(
        manager.get_url("DB_PASSWORD", &settings).unwrap(),
        "http://127.0.0.1:8200/v1/teams/team-b/kv-prod/data/virtual-keys/DB_PASSWORD"
    );
}

#[test]
fn empty_overrides_remove_the_segments() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
        ("HCP_VAULT_MOUNT_NAME", "kv-prod"),
        ("HCP_VAULT_PATH_PREFIX", "myapp"),
    ]);
    let settings = VaultSecretSettings {
        namespace: Some(String::new()),
        mount: Some(String::new()),
        path_prefix: Some(String::new()),
        data: None,
    };
    assert_eq!(
        manager.get_url("DB_PASSWORD", &settings).unwrap(),
        "http://127.0.0.1:8200/v1/secret/data/DB_PASSWORD"
    );
}

#[test]
fn whitespace_and_slash_only_overrides_remove_the_segments() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
    ]);
    let settings = VaultSecretSettings {
        namespace: Some(" / ".to_owned()),
        mount: Some(" // ".to_owned()),
        path_prefix: Some(" / ".to_owned()),
        data: None,
    };
    assert_eq!(
        manager.get_url("DB_PASSWORD", &settings).unwrap(),
        "http://127.0.0.1:8200/v1/secret/data/DB_PASSWORD"
    );
}

#[test]
fn a_null_override_uses_the_manager_value() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
        ("HCP_VAULT_MOUNT_NAME", "kv-prod"),
        ("HCP_VAULT_PATH_PREFIX", "myapp"),
    ]);
    let params: VaultSecretParams =
        serde_json::from_str(r#"{"namespace":null,"mount":null,"path_prefix":null,"data":null}"#)
            .unwrap();
    let target = manager
        .build_secret_target("DB_PASSWORD", Some(&params))
        .unwrap();
    assert_eq!(
        target.url,
        "http://127.0.0.1:8200/v1/admin/kv-prod/data/myapp/DB_PASSWORD"
    );
    assert_eq!(target.data_key, "key");
    assert_eq!(target.secret_name, "DB_PASSWORD");
}

#[test]
fn nested_settings_build_the_target() {
    let manager = manager(TOKEN);
    let params: VaultSecretParams = serde_json::from_str(
        r#"{"secret_manager_settings":{"namespace":"teams/team-b","mount":"kv-prod","path_prefix":"virtual-keys","data":"password"}}"#,
    )
    .unwrap();
    let target = manager
        .build_secret_target("DB_PASSWORD", Some(&params))
        .unwrap();
    assert_eq!(
        target.url,
        "http://127.0.0.1:8200/v1/teams/team-b/kv-prod/data/virtual-keys/DB_PASSWORD"
    );
    assert_eq!(target.data_key, "password");
}

#[test]
fn flat_settings_build_the_target() {
    let manager = manager(TOKEN);
    let params: VaultSecretParams =
        serde_json::from_str(r#"{"namespace":"teams/team-a","mount":"kv-team"}"#).unwrap();
    assert_eq!(params.secret_manager_settings, None);
    let target = manager
        .build_secret_target("DB_PASSWORD", Some(&params))
        .unwrap();
    assert_eq!(
        target.url,
        "http://127.0.0.1:8200/v1/teams/team-a/kv-team/data/DB_PASSWORD"
    );
}

#[test]
fn a_present_nested_settings_block_replaces_the_flat_keys() {
    let manager = manager(TOKEN);
    let params: VaultSecretParams = serde_json::from_str(
        r#"{"secret_manager_settings":{},"namespace":"teams/team-a","mount":"kv-team"}"#,
    )
    .unwrap();
    let target = manager
        .build_secret_target("DB_PASSWORD", Some(&params))
        .unwrap();
    assert_eq!(
        target.url,
        "http://127.0.0.1:8200/v1/secret/data/DB_PASSWORD"
    );
}

#[test]
fn a_null_nested_settings_block_falls_back_to_the_flat_keys() {
    let manager = manager(TOKEN);
    let params: VaultSecretParams =
        serde_json::from_str(r#"{"secret_manager_settings":null,"namespace":"teams/team-a"}"#)
            .unwrap();
    let target = manager
        .build_secret_target("DB_PASSWORD", Some(&params))
        .unwrap();
    assert_eq!(
        target.url,
        "http://127.0.0.1:8200/v1/teams/team-a/secret/data/DB_PASSWORD"
    );
}

#[test]
fn other_keys_in_the_params_are_ignored() {
    let manager = manager(TOKEN);
    let params: VaultSecretParams =
        serde_json::from_str(r#"{"namespace":"teams/team-a","unrelated":true}"#).unwrap();
    assert_eq!(
        manager
            .build_secret_target("DB_PASSWORD", Some(&params))
            .unwrap()
            .url,
        "http://127.0.0.1:8200/v1/teams/team-a/secret/data/DB_PASSWORD"
    );
}

#[test]
fn no_params_use_the_manager_values() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
    ]);
    let target = manager.build_secret_target("DB_PASSWORD", None).unwrap();
    assert_eq!(
        target.url,
        "http://127.0.0.1:8200/v1/admin/secret/data/DB_PASSWORD"
    );
}

#[rstest]
#[case::explicit_override("password", "password")]
#[case::empty_override("", "key")]
#[case::blank_override("   ", "key")]
#[case::slashes_are_kept(" /a/ ", "/a/")]
fn the_data_key_is_trimmed_of_whitespace_only(#[case] data: &str, #[case] expected: &str) {
    let manager = manager(TOKEN);
    let params = VaultSecretParams {
        secret_manager_settings: Some(VaultSecretSettings {
            namespace: None,
            mount: None,
            path_prefix: None,
            data: Some(data.to_owned()),
        }),
        flat: VaultSecretSettings {
            namespace: None,
            mount: None,
            path_prefix: None,
            data: None,
        },
    };
    let target = manager
        .build_secret_target("DB_PASSWORD", Some(&params))
        .unwrap();
    assert_eq!(target.data_key, expected);
}

#[rstest]
#[case::parent_traversal("../secrets")]
#[case::inner_parent_traversal("apps/../secrets")]
#[case::only_parent("..")]
#[case::control_character("apps/\u{1}secret")]
fn unsafe_secret_names_are_refused(#[case] name: &str) {
    let manager = manager(TOKEN);
    drop(manager.get_url(name, &default_settings()).unwrap_err());
    drop(manager.build_secret_target(name, None).unwrap_err());
}

#[test]
fn the_manager_exposes_its_address_and_namespaces() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_ADDR", "http://vault.test:8200"),
        ("HCP_VAULT_NAMESPACE", "admin"),
        ("HCP_VAULT_LOGIN_NAMESPACE", "root"),
        ("HCP_VAULT_SECRET_NAMESPACE", "teams/team-a"),
    ]);
    assert_eq!(manager.vault_addr(), "http://vault.test:8200");
    assert_eq!(manager.vault_login_namespace(), Some("root"));
    assert_eq!(manager.vault_secret_namespace(), Some("teams/team-a"));
}

#[test]
fn the_namespaces_fall_back_to_the_global_variable() {
    let manager = manager(&[
        ("HCP_VAULT_TOKEN", "token-1"),
        ("HCP_VAULT_NAMESPACE", "admin"),
    ]);
    assert_eq!(manager.vault_login_namespace(), Some("admin"));
    assert_eq!(manager.vault_secret_namespace(), Some("admin"));
}

#[test]
fn without_a_namespace_both_are_none() {
    let manager = manager(TOKEN);
    assert_eq!(manager.vault_login_namespace(), None);
    assert_eq!(manager.vault_secret_namespace(), None);
}
