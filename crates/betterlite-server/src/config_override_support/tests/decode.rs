#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::{object, value};
use crate::config_override_support::decode::{
    OverrideConfig, OverrideDecodeError, StoredOverrideError, decode_config_override,
    decode_stored_override,
};
use crate::config_override_support::models::OverrideKind;

#[test]
fn reads_string_fields_and_ignores_unknown_ones() {
    let decoded = decode_config_override(
        br#"{"vault_addr":"https://vault.example.com:8200","vault_token":null,"vault_namespace":"","unknown_field":"ignored"}"#,
        OverrideKind::HashicorpVault,
    )
    .unwrap();
    assert_eq!(
        decoded.values,
        object(
            r#"{
                "vault_addr": "https://vault.example.com:8200",
                "vault_namespace": ""
            }"#
        )
    );
    match decoded.config {
        OverrideConfig::HashicorpVault(model) => {
            assert_eq!(model.vault_token, None);
            assert_eq!(model.vault_namespace.as_deref(), Some(""));
        }
        OverrideConfig::CyberArk(_) => panic!("a vault body decoded as a CyberArk request"),
    }
}

#[test]
fn names_the_field_with_a_non_string_value() {
    let Err(error) = decode_config_override(br#"{"vault_addr":7}"#, OverrideKind::HashicorpVault)
    else {
        panic!("a non-string field value decoded");
    };
    let message = error.to_string();
    assert!(message.contains("vault_addr"));
    assert!(message.contains("hashicorp_vault"));
    assert!(message.contains("expected a string"));
}

#[test]
fn rejects_a_body_that_is_not_an_object() {
    assert!(matches!(
        decode_config_override(b"42", OverrideKind::CyberArk),
        Err(OverrideDecodeError::NotAnObject)
    ));
    assert!(matches!(
        decode_config_override(b"{", OverrideKind::HashicorpVault),
        Err(OverrideDecodeError::Json(_))
    ));
}

#[test]
fn keeps_a_cyberark_body_out_of_the_vault_mapping() {
    let body = br#"{"cyberark_api_base":"https://conjur.example.com","refresh_interval":"60","ssl_verify":null}"#;
    let cyberark = decode_config_override(body, OverrideKind::CyberArk).unwrap();
    assert_eq!(
        cyberark.values,
        object(
            r#"{
                "cyberark_api_base": "https://conjur.example.com",
                "refresh_interval": "60"
            }"#
        )
    );
    assert!(matches!(cyberark.config, OverrideConfig::CyberArk(_)));
    assert!(
        decode_config_override(body, OverrideKind::HashicorpVault)
            .unwrap()
            .values
            .is_empty()
    );
}

#[test]
fn stored_override_reads_json_text_and_parsed_objects() {
    let text = JsonValue::String(r#"{"vault_addr":"https://vault.example.com"}"#.to_owned());
    assert_eq!(
        decode_stored_override(&text, OverrideKind::HashicorpVault).unwrap(),
        object(r#"{"vault_addr": "https://vault.example.com"}"#)
    );
    let parsed = value(r#"{"vault_token": 5, "approle_role_id": null}"#);
    assert_eq!(
        decode_stored_override(&parsed, OverrideKind::HashicorpVault).unwrap(),
        object(r#"{"vault_token": 5, "approle_role_id": null}"#)
    );
}

#[test]
fn stored_override_turns_malformed_json_into_an_empty_mapping() {
    let malformed = JsonValue::String("{not json".to_owned());
    assert!(
        decode_stored_override(&malformed, OverrideKind::HashicorpVault)
            .unwrap()
            .is_empty()
    );
    let empty = JsonValue::String(String::new());
    assert!(
        decode_stored_override(&empty, OverrideKind::CyberArk)
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::json_text_scalar(JsonValue::String("42".to_owned()))]
#[case::number(JsonValue::from(42_i32))]
#[case::array(JsonValue::Array(vec![JsonValue::from(1_i32)]))]
#[case::boolean(JsonValue::from(true))]
#[case::null(JsonValue::Null)]
fn stored_override_keeps_a_valid_non_object_as_a_shape_failure(#[case] stored: JsonValue) {
    assert!(matches!(
        decode_stored_override(&stored, OverrideKind::CyberArk),
        Err(StoredOverrideError::NotAnObject { .. })
    ));
}
