use betterlite_logging::private_json::error::PrivateJsonError;

use super::support::object;
use crate::coordination_redis_settings::decode::{
    DecodeError, decode_coordination_settings, decode_persisted_coordination_settings,
};

#[test]
fn decodes_the_settings_object_of_a_request_body() {
    assert_eq!(
        decode_coordination_settings(br#"{"settings": {"host": "h", "port": 6379}}"#).unwrap(),
        object(r#"{"host": "h", "port": 6379}"#),
    );
}

#[test]
fn rejects_a_body_without_a_settings_object() {
    assert!(matches!(
        decode_coordination_settings(b"[]"),
        Err(DecodeError::NotAnObject)
    ));
    assert!(matches!(
        decode_coordination_settings(b"{}"),
        Err(DecodeError::MissingSettings)
    ));
    assert!(matches!(
        decode_coordination_settings(br#"{"settings": null}"#),
        Err(DecodeError::SettingsNotAnObject)
    ));
    assert!(matches!(
        decode_coordination_settings(br#"{"settings": []}"#),
        Err(DecodeError::SettingsNotAnObject)
    ));
    assert!(matches!(
        decode_coordination_settings(br#"{"settings": "#),
        Err(DecodeError::Bounded(_))
    ));
}

#[test]
fn decodes_the_persisted_block_of_a_general_settings_row() {
    assert_eq!(
        decode_persisted_coordination_settings(
            br#"{"coordination_redis": {}, "master_key": "sk-x"}"#,
        )
        .unwrap(),
        Some(object("{}")),
    );
    assert_eq!(
        decode_persisted_coordination_settings(br#"{"coordination_redis": "text"}"#).unwrap(),
        None,
    );
    assert_eq!(
        decode_persisted_coordination_settings(br#"{"master_key": "sk-x"}"#).unwrap(),
        None,
    );
    assert!(matches!(
        decode_persisted_coordination_settings(br#""text""#),
        Err(DecodeError::NotAnObject)
    ));
}

#[test]
fn both_decoders_hold_the_byte_and_depth_bounds() {
    let oversized = format!(r#"{{"settings": "{}"}}"#, "a".repeat(0x0010_0000));
    assert!(matches!(
        decode_coordination_settings(oversized.as_bytes()),
        Err(DecodeError::Bounded(PrivateJsonError::TooLarge(..)))
    ));
    assert!(matches!(
        decode_persisted_coordination_settings(oversized.as_bytes()),
        Err(DecodeError::Bounded(PrivateJsonError::TooLarge(..)))
    ));
    let deep = format!("{}{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_coordination_settings(deep.as_bytes()),
        Err(DecodeError::Bounded(PrivateJsonError::TooDeep(_)))
    ));
    assert!(matches!(
        decode_persisted_coordination_settings(deep.as_bytes()),
        Err(DecodeError::Bounded(PrivateJsonError::TooDeep(_)))
    ));
}
