#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;

use super::{Fixture, fixture, object, regenerate_request};
use crate::key_policy_fields::regenerate::regenerate_as_update_request;
use crate::key_policy_fields::rotation::{
    ALIAS_REQUIRED_MESSAGE, RotationError, RotationSwitches, set_key_rotation_fields,
};

const ROTATION_DISABLED: RotationSwitches = RotationSwitches {
    auto_rotate: false,
    store_virtual_keys: true,
};
const ROTATION_STORED: RotationSwitches = RotationSwitches {
    auto_rotate: true,
    store_virtual_keys: true,
};
const ROTATION_UNSTORED: RotationSwitches = RotationSwitches {
    auto_rotate: true,
    store_virtual_keys: false,
};

#[test]
fn disabled_rotation_keeps_fields() {
    let fixture = fixture();
    let clock = fixture.clock();
    let before = object(
        r#"{"auto_rotate":true,"rotation_interval":"1d","key_rotation_at":"2026-01-01T00:00:00Z"}"#,
    );
    let mut data = before.clone();
    set_key_rotation_fields(&mut data, ROTATION_DISABLED, Some("1h"), None, clock).unwrap();
    assert_eq!(data, before);
    // switching rotation on without an interval changes nothing either
    set_key_rotation_fields(&mut data, ROTATION_STORED, Some(""), None, clock).unwrap();
    assert_eq!(data, before);
    set_key_rotation_fields(&mut data, ROTATION_STORED, None, None, clock).unwrap();
    assert_eq!(data, before);
}

#[test]
fn stored_rotation_requires_alias() {
    let fixture = fixture();
    let clock = fixture.clock();
    let mut data = object("{}");
    let error =
        set_key_rotation_fields(&mut data, ROTATION_STORED, Some("30d"), None, clock).unwrap_err();
    let RotationError::AliasRequired(exception) = error else {
        panic!("a missing alias is the rotation failure");
    };
    assert_eq!(exception.message, ALIAS_REQUIRED_MESSAGE);
    assert_eq!(exception.code, "400");
    assert_eq!(exception.error_type, "bad_request_error");
    assert_eq!(exception.param.as_deref(), Some("key_alias"));
    // an explicit null is missing too
    let mut null_alias = object(r#"{"key_alias":null}"#);
    assert!(matches!(
        set_key_rotation_fields(&mut null_alias, ROTATION_STORED, Some("30d"), None, clock),
        Err(RotationError::AliasRequired(_))
    ));
    assert_alias_requirement_cleared(&fixture);
}

// the stored alias, a requested alias or store_virtual_keys off all clear the requirement
fn assert_alias_requirement_cleared(fixture: &Fixture) {
    let clock = fixture.clock();
    let mut stored = object("{}");
    set_key_rotation_fields(
        &mut stored,
        ROTATION_STORED,
        Some("30d"),
        Some("existing"),
        clock,
    )
    .unwrap();
    assert_eq!(
        stored.get("key_rotation_at"),
        Some(&JsonData::String("2026-05-31T10:00:00.123456Z".to_owned()))
    );
    assert_eq!(
        stored.get("rotation_interval"),
        Some(&JsonData::String("30d".to_owned()))
    );
    assert_eq!(stored.get("auto_rotate"), Some(&JsonData::Bool(true)));
    let mut requested = object(r#"{"key_alias":"alias"}"#);
    set_key_rotation_fields(&mut requested, ROTATION_STORED, Some("30d"), None, clock).unwrap();
    let mut without_store = object("{}");
    set_key_rotation_fields(
        &mut without_store,
        ROTATION_UNSTORED,
        Some("30d"),
        None,
        clock,
    )
    .unwrap();
    assert_eq!(
        without_store.get("auto_rotate"),
        Some(&JsonData::Bool(true))
    );
}

#[test]
fn empty_alias_is_present() {
    let fixture = fixture();
    let clock = fixture.clock();
    let mut data = object(r#"{"key_alias":""}"#);
    set_key_rotation_fields(&mut data, ROTATION_STORED, Some("1h"), None, clock).unwrap();
    assert_eq!(
        data.get("key_rotation_at"),
        Some(&JsonData::String("2026-05-01T11:00:00.123456Z".to_owned()))
    );
}

#[test]
fn regeneration_absent_null_empty_duration() {
    let fields_only = regenerate_request(
        r#"{"grace_period":"1h","key_type":"llm_api","send_invite_email":true}"#,
    );
    assert!(
        regenerate_as_update_request("hashed", &fields_only)
            .unwrap()
            .is_none()
    );
    let request = regenerate_request(
        r#"{"duration":"","budget_duration":"","metadata":null,"key":"caller"}"#,
    );
    let cleared_update = regenerate_as_update_request("hashed", &request)
        .unwrap()
        .expect("clearing metadata is a change");
    let cleared = cleared_update.project().unwrap().supplied;
    assert_eq!(cleared.get("metadata"), Some(&JsonData::Null));
    assert_eq!(
        cleared.get("key"),
        Some(&JsonData::String("hashed".to_owned()))
    );
    assert!(!cleared.contains_key("duration"));
    assert!(!cleared.contains_key("budget_duration"));
    // a nonempty duration is a real change and survives the patch
    let timed = regenerate_request(r#"{"duration":"1h"}"#);
    let timed_update = regenerate_as_update_request("hashed", &timed)
        .unwrap()
        .expect("a duration is a change");
    assert_eq!(
        timed_update.project().unwrap().supplied.get("duration"),
        Some(&JsonData::String("1h".to_owned()))
    );
}
