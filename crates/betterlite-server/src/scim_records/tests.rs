#![cfg(test)]

use super::{decode_scim_resource, models::DecodedScimResource, models::ResourceKind};
use std::fs;

#[test]
fn omitted_active_defaults_but_is_not_present() {
    let decoded =
        decode_scim_resource(br#"{"schemas":["u"],"userName":"Ada"}"#, ResourceKind::User).unwrap();
    let DecodedScimResource::User(user) = decoded else {
        panic!("wrong resource")
    };
    assert!(user.active);
    assert!(!user.active_present);
}

#[test]
fn explicit_active_true_is_present() {
    let decoded =
        decode_scim_resource(br#"{"schemas":["u"],"active":true}"#, ResourceKind::User).unwrap();
    let DecodedScimResource::User(user) = decoded else {
        panic!("wrong resource")
    };
    assert!(user.active && user.active_present);
}

#[test]
fn patch_operation_lowercases_without_trimming() {
    let decoded =
        decode_scim_resource(br#"{"Operations":[{"op":"RePlAcE"}]}"#, ResourceKind::Patch).unwrap();
    let DecodedScimResource::Patch(patch) = decoded else {
        panic!("wrong resource")
    };
    assert_eq!(patch.operations[0].op, "replace");
    assert_eq!(patch.schemas, vec![super::models::PATCH_SCHEMA]);
    let result = decode_scim_resource(br#"{"Operations":[{"op":" add"}]}"#, ResourceKind::Patch);
    result.unwrap_err();
}

#[test]
fn malformed_member_type_is_absent() {
    let decoded = decode_scim_resource(
        br#"{"schemas":["g"],"displayName":"G","members":[{"value":"1","type":3}]}"#,
        ResourceKind::Group,
    )
    .unwrap();
    let DecodedScimResource::Group(group) = decoded else {
        panic!("wrong resource")
    };
    assert_eq!(group.members.unwrap()[0].kind, None);
}

#[test]
fn enterprise_ref_accepts_both_names() {
    let bytes = br#"{"schemas":["u"],"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"manager":{"$ref":"/Users/1"}}}"#;
    let decoded = decode_scim_resource(bytes, ResourceKind::User).unwrap();
    let DecodedScimResource::User(user) = decoded else {
        panic!("wrong resource")
    };
    assert_eq!(
        user.enterprise_user
            .unwrap()
            .manager
            .unwrap()
            .reference
            .as_deref(),
        Some("/Users/1")
    );
}

#[test]
fn replays_the_scim_resource_corpus() {
    for entry in fs::read_dir("fuzz/corpus/scim_resource").unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(path).unwrap();
        for kind in [ResourceKind::User, ResourceKind::Group, ResourceKind::Patch] {
            drop(decode_scim_resource(&bytes, kind));
        }
    }
}
