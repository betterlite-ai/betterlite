#![cfg(test)]
use crate::scim_records::discovery::{ResourceType, ServiceProviderConfig};
use crate::scim_records::lax_bool::ScimBool;
use crate::scim_records::user::{Active, User};
use crate::scim_records::{ResourceKind, decode_scim_resource};
use rstest::rstest;
use std::fs;
use std::path::PathBuf;

fn dump(body: &str, kind: ResourceKind) -> String {
    serde_json::to_string(&decode_scim_resource(body.as_bytes(), kind).unwrap()).unwrap()
}
#[rstest]
#[case::omitted(r#"{"schemas":[]}"#, Active::Defaulted(ScimBool(true)))]
#[case::explicit_true(r#"{"schemas":[],"active":true}"#, Active::Sent(ScimBool(true)))]
#[case::explicit_false(r#"{"schemas":[],"active":false}"#, Active::Sent(ScimBool(false)))]
#[case::lax_text(r#"{"schemas":[],"active":"False"}"#, Active::Sent(ScimBool(false)))]
#[case::lax_number(r#"{"schemas":[],"active":1}"#, Active::Sent(ScimBool(true)))]
fn active_keeps_whether_it_was_sent(#[case] body: &str, #[case] active: Active) {
    let user: User = serde_json::from_str(body).unwrap();
    assert_eq!(user.active, active);
}
#[rstest]
#[case::null_active(r#"{"schemas":[],"active":null}"#, ResourceKind::User)]
#[case::number_two_active(r#"{"schemas":[],"active":2}"#, ResourceKind::User)]
#[case::user_without_schemas(r#"{"userName":"ada"}"#, ResourceKind::User)]
#[case::group_without_name(r#"{"schemas":[]}"#, ResourceKind::Group)]
#[case::padded_op(r#"{"Operations":[{"op":" add"}]}"#, ResourceKind::Patch)]
#[case::unknown_op(r#"{"Operations":[{"op":"delete"}]}"#, ResourceKind::Patch)]
#[case::number_op(r#"{"Operations":[{"op":1}]}"#, ResourceKind::Patch)]
#[case::missing_operations(r#"{"schemas":[]}"#, ResourceKind::Patch)]
#[case::attribute_number(r#"{"schemas":[],"roles":[7]}"#, ResourceKind::User)]
#[case::malformed_json(r#"{"schemas":[]"#, ResourceKind::User)]
fn rejects_what_pydantic_rejects(#[case] body: &str, #[case] kind: ResourceKind) {
    decode_scim_resource(body.as_bytes(), kind).unwrap_err();
}
#[rstest]
#[case::user_meta(
    r#"{"schemas":[],"meta":{"directory":1,"directory":2}}"#,
    ResourceKind::User
)]
#[case::group_meta(
    r#"{"schemas":[],"displayName":"group","meta":{"directory":1,"directory":2}}"#,
    ResourceKind::Group
)]
#[case::entitlement(
    r#"{"schemas":[],"entitlements":[{"directory":1,"directory":2}]}"#,
    ResourceKind::User
)]
#[case::role(
    r#"{"schemas":[],"roles":[{"directory":1,"directory":2}]}"#,
    ResourceKind::User
)]
fn rejects_duplicate_passthrough_keys(#[case] body: &str, #[case] kind: ResourceKind) {
    let error = decode_scim_resource(body.as_bytes(), kind).unwrap_err();
    assert!(
        error.to_string().contains("duplicate key: directory"),
        "{error}"
    );
}
#[rstest]
#[case::absent_blocks(
    ResourceKind::User,
    r#"{"schemas":[],"groups":[{"value":"g1"},{"value":"g2","type":null}]}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"userName":null,"name":null,"displayName":null,"active":true,"emails":null,"groups":[{"value":"g1","display":null,"type":"direct"},{"value":"g2","display":null,"type":null}]}"#
)]
#[case::bare_and_extra_attributes(
    ResourceKind::User,
    r#"{"schemas":[],"entitlements":["admin",{"value":"b","source":{"hr":[1]},"primary":"yes"}],"roles":[]}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"userName":null,"name":null,"displayName":null,"active":true,"emails":null,"groups":null,"entitlements":[{"value":"admin","display":null,"type":null,"primary":null},{"value":"b","display":null,"type":null,"primary":true,"source":{"hr":[1]}}],"roles":[]}"#
)]
#[case::patch_mixed_values(
    ResourceKind::Patch,
    r#"{"Operations":[{"op":"Add","path":"members","value":[{"value":"u1"},"u2",3]},{"op":"REPLACE","value":{"active":"False"}},{"op":"remove","value":null}]}"#,
    r#"{"schemas":["urn:ietf:params:scim:api:messages:2.0:PatchOp"],"Operations":[{"op":"add","path":"members","value":[{"value":"u1"},"u2",3]},{"op":"replace","path":null,"value":{"active":"False"}},{"op":"remove","path":null,"value":null}]}"#
)]
#[case::enterprise_urn_and_ref(
    ResourceKind::User,
    r#"{"schemas":[],"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"department":"R&D","manager":{"$ref":"/Users/1"}}}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"userName":null,"name":null,"displayName":null,"active":true,"emails":null,"groups":null,"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"employeeNumber":null,"costCenter":null,"organization":null,"division":null,"department":"R&D","manager":{"value":null,"displayName":null,"$ref":"/Users/1"}}}"#
)]
fn dumps_match_pydantic(#[case] kind: ResourceKind, #[case] body: &str, #[case] expected: &str) {
    assert_eq!(dump(body, kind), expected);
}
#[test]
fn resource_type_renames_only_its_own_schema_field() {
    let body = r#"{"id":"User","name":"User","endpoint":"/Users","schema_":"core","schemaExtensions":[{"schema_":"ext","required":false}]}"#;
    let record: ResourceType = serde_json::from_str(body).unwrap();
    let expected = r#"{"schemas":["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],"id":"User","name":"User","description":null,"endpoint":"/Users","schema":"core","schemaExtensions":[{"schema_":"ext","required":false}],"meta":null}"#;
    assert_eq!(serde_json::to_string(&record).unwrap(), expected);
}
#[test]
fn service_provider_config_supports_only_patch() {
    let off = r#"{"supported":false,"maxOperations":null,"maxPayloadSize":null,"maxResults":null}"#;
    let expected = format!(
        r#"{{"schemas":["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],"patch":{{"supported":true,"maxOperations":null,"maxPayloadSize":null,"maxResults":null}},"bulk":{off},"filter":{off},"changePassword":{off},"sort":{off},"etag":{off},"authenticationSchemes":null,"meta":null}}"#
    );
    let dumped = serde_json::to_string(&ServiceProviderConfig::default());
    assert_eq!(dumped.unwrap(), expected);
}
#[rstest]
fn replays_the_scim_resource_corpus(#[files("fuzz/corpus/scim_resource/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let kind = match bytes.first() {
        Some(b'u') => ResourceKind::User,
        Some(b'g') => ResourceKind::Group,
        _ => ResourceKind::Patch,
    };
    let body = bytes.get(1..).unwrap_or_default();
    drop(decode_scim_resource(body, kind));
}
