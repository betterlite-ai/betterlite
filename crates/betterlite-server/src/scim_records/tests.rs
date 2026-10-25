#![cfg(test)]
use crate::scim_records::{
    ScimRecordError, decode_scim_resource,
    discovery::{ScimResourceType, ScimSchema, ScimSchemaAttribute},
    models::{DecodedScimResource, PATCH_SCHEMA, ResourceKind},
};
use std::{fs, path::Path};
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
    assert_eq!(patch.schemas, vec![PATCH_SCHEMA]);
    let result = decode_scim_resource(br#"{"Operations":[{"op":" add"}]}"#, ResourceKind::Patch);
    assert!(matches!(result, Err(ScimRecordError::UnsupportedOp { .. })));
}
#[test]
fn discovery_dumps_remove_null_nested_blocks() {
    let attribute = ScimSchemaAttribute {
        name: "manager".to_owned(),
        kind: "complex".to_owned(),
        multi_valued: false,
        description: None,
        required: false,
        mutability: "readWrite".to_owned(),
        returned: "default".to_owned(),
        uniqueness: "none".to_owned(),
        sub_attributes: None,
    };
    let schema = ScimSchema {
        schemas: vec!["schema".to_owned()],
        id: "User".to_owned(),
        name: "User".to_owned(),
        description: None,
        attributes: vec![attribute],
        meta: None,
    };
    let dumped = serde_json::to_value(schema).unwrap();
    assert!(dumped["attributes"][0].get("subAttributes").is_none());
    let resource = ScimResourceType {
        schemas: vec!["resource".to_owned()],
        id: "User".to_owned(),
        name: "User".to_owned(),
        description: None,
        endpoint: "/Users".to_owned(),
        schema_: "schema".to_owned(),
        schema_extensions: None,
        meta: None,
    };
    let resource_dump = serde_json::to_value(resource).unwrap();
    assert!(resource_dump.get("schemaExtensions").is_none());
    assert_eq!(resource_dump["schema"], "schema");
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
fn enterprise_alias_and_manager_ref_are_preserved() {
    let bytes = br#"{"schemas":["u"],"enterprise_user":{"manager":{"ref":"/Users/2"}}}"#;
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
        Some("/Users/2")
    );
}
#[test]
fn emailstr_normalizes_domain_and_rejects_invalid_values() {
    let decoded = decode_scim_resource(
        br#"{"schemas":["u"],"emails":[{"value":"Ada@EXAMPLE.COM"}]}"#,
        ResourceKind::User,
    )
    .unwrap();
    let DecodedScimResource::User(user) = decoded else {
        panic!("wrong resource")
    };
    assert_eq!(user.emails.unwrap()[0].value, "Ada@example.com");
    decode_scim_resource(
        br#"{"schemas":["u"],"emails":[{"value":"invalid"}]}"#,
        ResourceKind::User,
    )
    .unwrap_err();
}
#[test]
fn group_type_defaults_only_when_omitted() {
    let absent_result = decode_scim_resource(
        br#"{"schemas":["g"],"displayName":"G","groups":[{"value":"1"}]}"#,
        ResourceKind::User,
    )
    .unwrap();
    let explicit_result = decode_scim_resource(
        br#"{"schemas":["g"],"displayName":"G","groups":[{"value":"1","type":null}]}"#,
        ResourceKind::User,
    )
    .unwrap();
    let DecodedScimResource::User(absent_user) = absent_result else {
        panic!("wrong resource")
    };
    let DecodedScimResource::User(explicit_user) = explicit_result else {
        panic!("wrong resource")
    };
    assert_eq!(
        absent_user.groups.unwrap()[0].kind.as_deref(),
        Some("direct")
    );
    assert_eq!(explicit_user.groups.unwrap()[0].kind, None);
}
#[test]
fn replays_the_scim_resource_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/scim_resource");
    for entry in fs::read_dir(corpus).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        for kind in [ResourceKind::User, ResourceKind::Group, ResourceKind::Patch] {
            drop(decode_scim_resource(&bytes, kind));
        }
    }
}
