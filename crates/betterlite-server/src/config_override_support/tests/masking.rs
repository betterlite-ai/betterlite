#![cfg(test)]

use super::object;
use crate::config_override_support::masking::mask_sensitive_fields;
use crate::config_override_support::models::OverrideKind;

#[test]
fn masks_only_sensitive_string_values() {
    let masked = mask_sensitive_fields(
        OverrideKind::HashicorpVault,
        &object(
            r#"{
                "vault_addr": "https://vault.example.com:8200",
                "vault_token": "hunter2-token",
                "approle_secret_id": "hunter2-approle-key",
                "approle_role_id": 7,
                "client_cert": "cert-path",
                "client_key": "",
                "vault_namespace": null
            }"#,
        ),
    );
    assert_eq!(
        masked,
        object(
            r#"{
                "vault_addr": "https://vault.example.com:8200",
                "vault_token": "hunt*****oken",
                "approle_secret_id": "hunt***********-key",
                "approle_role_id": 7,
                "client_cert": "cert-path",
                "client_key": "",
                "vault_namespace": null
            }"#
        )
    );
}

#[test]
fn masks_the_cyberark_api_key_and_shared_client_key_only() {
    let masked = mask_sensitive_fields(
        OverrideKind::CyberArk,
        &object(
            r#"{
                "cyberark_api_base": "https://conjur.example.com",
                "cyberark_api_key": "hunter2-api-key",
                "client_key": "short",
                "client_cert": "cert-path",
                "ssl_verify": "false"
            }"#,
        ),
    );
    assert_eq!(
        masked,
        object(
            r#"{
                "cyberark_api_base": "https://conjur.example.com",
                "cyberark_api_key": "hunt*******-key",
                "client_key": "*****",
                "client_cert": "cert-path",
                "ssl_verify": "false"
            }"#
        )
    );
}
